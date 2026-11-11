//! `thumb://localhost/<image id>` — library thumbnails, generated on first request and cached
//! on disk. The webview loads them like any image, so off-screen cells cost nothing.
//!
//! Photos that have been opened in Edit show their current version instead: Edit renders it
//! with the preview shader and saves it here (`library_save_edited`).

use std::io::Cursor;
use std::path::Path;
use std::sync::OnceLock;

use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::history::CatalogState;
use crate::store::Store;

pub const SCHEME: &str = "thumb";
const EDGE: u32 = 640;

/// Thumbnail work is IO + JPEG decode heavy; a small dedicated pool keeps the UI responsive
/// and bounds memory when the grid asks for hundreds at once.
fn pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 6);
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("thumb-{i}"))
            .start_handler(|_| crate::priority::lower_this_thread())
            .build()
            .unwrap()
    })
}

fn respond(responder: UriSchemeResponder, status: StatusCode, body: Vec<u8>) {
    let resp = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, if status.is_success() { "image/jpeg" } else { "text/plain" })
        // Errors aren't cached: a thumbnail that failed (still importing, say) is retried.
        .header(header::CACHE_CONTROL, if status.is_success() { "max-age=31536000, immutable" } else { "no-store" })
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(body)
        .unwrap();
    responder.respond(resp);
}

fn generate(src: &Path, dest: &Path) -> Result<Vec<u8>, String> {
    let img = tonality_raw::thumbnail(src, EDGE).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut buf), 85)
        .encode_image(&img)
        .map_err(|e| e.to_string())?;
    if let Some(dir) = dest.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Write-then-rename so a concurrent reader never sees a partial file.
    let tmp = crate::store::temp_path(dest);
    if std::fs::write(&tmp, &buf).is_err() || std::fs::rename(&tmp, dest).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(buf)
}

/// Save Edit's rendering of an image's current version (JPEG bytes in the request body,
/// image id in the `x-image-id` header).
#[tauri::command]
pub async fn library_save_edited(
    request: tauri::ipc::Request<'_>,
    store: tauri::State<'_, Store>,
    catalog: tauri::State<'_, CatalogState>,
) -> crate::CmdResult<()> {
    let id: i64 = request
        .headers()
        .get("x-image-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or("missing x-image-id")?;
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected JPEG bytes".into());
    };
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return Err("not a JPEG".into());
    }
    let bytes = bytes.clone();
    let (_, hash, _) = catalog.read(move |c| c.image_file(id)).await?;
    let dest = store.edited_thumb_path(&hash);
    crate::blocking(move || {
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = crate::store::temp_path(&dest);
        std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &dest)).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    })
    .await
}

pub fn handle<R: Runtime>(ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let id: Option<i64> = request.uri().path().trim_matches('/').parse().ok();
    let Some(id) = id else {
        return respond(responder, StatusCode::BAD_REQUEST, b"bad thumbnail id".to_vec());
    };
    // Everything below (even the catalog lookup, which can wait on the catalog lock behind an
    // import) runs on the pool: this handler is called on the UI thread.
    pool().spawn(move || {
        let file = app.state::<CatalogState>().with(|c| c.image_file(id));
        let (rel, hash, _) = match file {
            Ok(f) => f,
            Err(e) => return respond(responder, StatusCode::NOT_FOUND, e.into_bytes()),
        };
        let store = app.state::<Store>();
        let path = store.abs(&rel);
        // Originals never change (they are content-addressed), so the hash is a stable cache key.
        let cache = store.thumb_path(&hash, EDGE);
        let edited = store.edited_thumb_path(&hash);
        if let Ok(bytes) = std::fs::read(&edited) {
            return respond(responder, StatusCode::OK, bytes);
        }
        if let Ok(bytes) = std::fs::read(&cache) {
            return respond(responder, StatusCode::OK, bytes);
        }
        match generate(&path, &cache) {
            Ok(bytes) => respond(responder, StatusCode::OK, bytes),
            Err(e) => {
                eprintln!("[thumb] {}: {e}", path.display());
                respond(responder, StatusCode::UNPROCESSABLE_ENTITY, e.into_bytes())
            }
        }
    });
}
