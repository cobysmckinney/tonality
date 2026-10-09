//! Searching and sorting the grid, on a library filled straight into its
//! database: the photos' files don't need to exist for this.

use tempfile::TempDir;
use tonality_lib::library::{FilmDetails, FilmPhoto, Library, NewPhoto, View};
use tonality_lib::search::{Facet, Field, Search, Sort, Term};

struct Shot {
    name: &'static str,
    taken: &'static str,
    make: Option<&'static str>,
    model: Option<&'static str>,
    lens: Option<&'static str>,
}

const SHOTS: [Shot; 5] = [
    Shot { name: "IMG_0001.CR2", taken: "2026-09-30T18:00:00", make: Some("Canon"), model: Some("EOS R5"), lens: Some("RF24-105mm F4 L IS USM") },
    Shot { name: "IMG_0002.CR2", taken: "2026-10-02T09:15:00", make: Some("Canon"), model: Some("Canon EOS R50"), lens: Some("RF50mm F1.8 STM") },
    Shot { name: "DSC09128.ARW", taken: "2025-10-12T12:00:00", make: Some("Sony"), model: Some("ILCE-7M3"), lens: Some("FE 35mm F1.8") },
    // Camera scans of film, shot on the R5 with a macro lens.
    Shot { name: "scan_01.CR2", taken: "2026-10-08T21:00:00", make: Some("Canon"), model: Some("EOS R5"), lens: Some("RF100mm F2.8 L MACRO IS USM") },
    Shot { name: "scan_02.CR2", taken: "2026-10-08T21:01:00", make: None, model: None, lens: None },
];

struct Fixture {
    _dir: TempDir,
    library: Library,
    /// The ids of `SHOTS`, in order.
    ids: Vec<i64>,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let library = Library::open(dir.path()).unwrap();
    let first = library.create_import("first card").unwrap();
    let second = library.create_import("film").unwrap();
    let ids: Vec<i64> = SHOTS
        .iter()
        .enumerate()
        .map(|(i, shot)| {
            library
                .insert_photo(&NewPhoto {
                    path: format!("Originals/{}", shot.name),
                    jpeg_path: None,
                    file_name: shot.name.to_string(),
                    kind: "raw",
                    fingerprint: format!("print-{i}"),
                    file_size: 1,
                    taken_at: shot.taken.to_string(),
                    width: None,
                    height: None,
                    make: shot.make.map(str::to_string),
                    model: shot.model.map(str::to_string),
                    lens: shot.lens.map(str::to_string),
                    iso: None,
                    aperture: None,
                    shutter: None,
                    focal_length: None,
                    import_id: if i < 3 { first } else { second },
                })
                .unwrap()
        })
        .collect();
    let film = |frame| FilmDetails {
        stock: Some("Kodak Portra 400".into()),
        iso: Some(400),
        camera: Some("Nikon FM2".into()),
        lens: Some("Nikkor 50mm f/1.4".into()),
        frame: Some(frame),
    };
    library
        .set_film_details(&[
            FilmPhoto { id: ids[3], taken_at: String::new(), file_name: String::new(), film: film(1) },
            FilmPhoto { id: ids[4], taken_at: String::new(), file_name: String::new(), film: film(2) },
        ])
        .unwrap();
    Fixture { _dir: dir, library, ids }
}

fn text(field: Field, text: &str) -> Term {
    Term { field, text: Some(text.into()), ..Default::default() }
}

fn exact(field: Field, text: &str) -> Term {
    Term { exact: true, ..self::text(field, text) }
}

fn span(from: &str, until: &str) -> Term {
    Term { from: Some(from.into()), until: Some(until.into()), ..Default::default() }
}

impl Fixture {
    /// Which of `SHOTS` the search finds in the library, by index, newest first.
    fn find(&self, terms: Vec<Term>) -> Vec<usize> {
        self.find_in(View::Library, terms, Sort::Newest)
    }

    fn find_in(&self, view: View, terms: Vec<Term>, sort: Sort) -> Vec<usize> {
        let found = self.library.find_photos(view, &Search { terms }, sort).unwrap();
        found.iter().map(|photo| self.ids.iter().position(|&id| id == photo.id).unwrap()).collect()
    }
}

#[test]
fn no_search_lists_the_whole_view() {
    let f = fixture();
    assert_eq!(f.find(vec![]), [4, 3, 1, 0, 2]);
}

#[test]
fn words_are_found_in_the_camera_lens_film_or_name() {
    let f = fixture();
    // The make and model together, wherever the model repeats the make.
    assert_eq!(f.find(vec![text(Field::Any, "canon eos")]), [3, 1, 0]);
    assert_eq!(f.find(vec![text(Field::Any, "ILCE")]), [2]);
    assert_eq!(f.find(vec![text(Field::Any, "macro")]), [3]);
    assert_eq!(f.find(vec![text(Field::Any, "portra")]), [4, 3]);
    // A film camera and lens count as the photo's own.
    assert_eq!(f.find(vec![text(Field::Any, "fm2")]), [4, 3]);
    assert_eq!(f.find(vec![text(Field::Any, "nikkor")]), [4, 3]);
    assert_eq!(f.find(vec![text(Field::Any, ".arw")]), [2]);
}

#[test]
fn every_term_has_to_match() {
    let f = fixture();
    assert_eq!(f.find(vec![text(Field::Any, "canon"), text(Field::Any, "portra")]), [3]);
    assert_eq!(f.find(vec![text(Field::Any, "sony"), text(Field::Any, "portra")]), Vec::<usize>::new());
}

#[test]
fn a_field_keeps_the_term_to_itself() {
    let f = fixture();
    // "R5" is in a file name nowhere, but "scan" is, and isn't a camera.
    assert_eq!(f.find(vec![text(Field::Camera, "scan")]), Vec::<usize>::new());
    assert_eq!(f.find(vec![text(Field::Name, "scan")]), [4, 3]);
    assert_eq!(f.find(vec![text(Field::Camera, "nikon")]), [4, 3]);
    assert_eq!(f.find(vec![text(Field::Lens, "50mm")]), [4, 3, 1]);
    assert_eq!(f.find(vec![text(Field::Film, "nikon")]), Vec::<usize>::new());
    assert_eq!(f.find(vec![text(Field::Film, "kodak")]), [4, 3]);
}

#[test]
fn an_exact_term_matches_the_whole_value_only() {
    let f = fixture();
    // The R5 but not the R50.
    assert_eq!(f.find(vec![text(Field::Camera, "Canon EOS R5")]), [3, 1, 0]);
    assert_eq!(f.find(vec![exact(Field::Camera, "canon eos r5")]), [3, 0]);
    assert_eq!(f.find(vec![exact(Field::Film, "Kodak Portra")]), Vec::<usize>::new());
    // Underscores and percent signs are taken as they are.
    assert_eq!(f.find(vec![exact(Field::Name, "IMG_0001.CR2")]), [0]);
    assert_eq!(f.find(vec![exact(Field::Name, "IMG%")]), Vec::<usize>::new());
    assert_eq!(f.find(vec![text(Field::Name, "G_0")]), [1, 0]);
}

#[test]
fn dates_match_by_span_or_month() {
    let f = fixture();
    assert_eq!(f.find(vec![span("2026-10", "2026-11")]), [4, 3, 1]);
    assert_eq!(f.find(vec![span("2026-10-08", "2026-10-09")]), [4, 3]);
    assert_eq!(f.find(vec![span("2025", "2026")]), [2]);
    // October of any year.
    assert_eq!(f.find(vec![Term { month: Some(10), ..Default::default() }]), [4, 3, 1, 2]);
    // From a time on, or before one.
    assert_eq!(f.find(vec![Term { from: Some("2026-10-02".into()), ..Default::default() }]), [4, 3, 1]);
    assert_eq!(f.find(vec![Term { until: Some("2026".into()), ..Default::default() }]), [2]);
}

#[test]
fn a_term_matches_by_its_text_or_its_date() {
    let f = fixture();
    // "2025" as typed: in a name, or the year a photo was taken.
    let year = Term { from: Some("2025".into()), until: Some("2026".into()), ..text(Field::Any, "2025") };
    assert_eq!(f.find(vec![year]), [2]);
    let month = Term { month: Some(9), ..text(Field::Any, "sep") };
    assert_eq!(f.find(vec![month]), [0]);
}

#[test]
fn a_term_that_asks_for_nothing_matches_nothing() {
    let f = fixture();
    assert_eq!(f.find(vec![Term::default()]), Vec::<usize>::new());
    assert_eq!(f.find(vec![text(Field::Any, "  ")]), Vec::<usize>::new());
}

#[test]
fn sorts_by_date_import_or_name() {
    let f = fixture();
    assert_eq!(f.find_in(View::Library, vec![], Sort::Oldest), [2, 0, 1, 3, 4]);
    assert_eq!(f.find_in(View::Library, vec![], Sort::Imported), [4, 3, 1, 0, 2]);
    assert_eq!(f.find_in(View::Library, vec![], Sort::Name), [2, 0, 1, 3, 4]);
    // The Imports view keeps its own order.
    assert_eq!(f.find_in(View::Imports, vec![], Sort::Name), [4, 3, 1, 0, 2]);
}

#[test]
fn searches_within_favorites_albums_and_recently_deleted() {
    let f = fixture();
    f.library.set_favorite(&[f.ids[0], f.ids[3]], true).unwrap();
    let album = f.library.create_album("Roll 12").unwrap();
    f.library.add_to_album(album, &[f.ids[2], f.ids[4]]).unwrap();
    f.library.trash(&[f.ids[1]]).unwrap();
    let portra = || vec![text(Field::Film, "portra")];
    assert_eq!(f.find_in(View::Favorites, portra(), Sort::Newest), [3]);
    assert_eq!(f.find_in(View::Album { id: album }, portra(), Sort::Newest), [4]);
    assert_eq!(f.find_in(View::Album { id: album }, vec![text(Field::Any, "sony")], Sort::Oldest), [2]);
    assert_eq!(f.find_in(View::Deleted, vec![text(Field::Any, "canon")], Sort::Newest), [1]);
    assert_eq!(f.find(vec![text(Field::Any, "R50")]), Vec::<usize>::new());
}

#[test]
fn facets_offer_what_the_view_holds() {
    let f = fixture();
    let facet = |value: &str, count| Facet { value: value.into(), count };
    let all = f.library.search_facets(View::Library).unwrap();
    assert_eq!(
        all.cameras,
        [facet("Canon EOS R5", 2), facet("Nikon FM2", 2), facet("Canon EOS R50", 1), facet("Sony ILCE-7M3", 1)]
    );
    assert_eq!(all.lenses[0], facet("Nikkor 50mm f/1.4", 2));
    assert_eq!(all.lenses.len(), 5);
    assert_eq!(all.films, [facet("Kodak Portra 400", 2)]);
    assert_eq!(all.months, [facet("2026-10", 3), facet("2026-09", 1), facet("2025-10", 1)]);

    let album = f.library.create_album("Sony").unwrap();
    f.library.add_to_album(album, &[f.ids[2]]).unwrap();
    let one = f.library.search_facets(View::Album { id: album }).unwrap();
    assert_eq!(one.cameras, [facet("Sony ILCE-7M3", 1)]);
    assert!(one.films.is_empty());

    // Photos in Recently Deleted are offered only there.
    f.library.trash(&[f.ids[2]]).unwrap();
    assert!(!f.library.search_facets(View::Library).unwrap().cameras.contains(&facet("Sony ILCE-7M3", 1)));
    assert_eq!(f.library.search_facets(View::Deleted).unwrap().cameras, [facet("Sony ILCE-7M3", 1)]);
}
