//! The models tonality can use, pinned by URL, size and checksum.

/// One downloadable file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFile {
    /// File name in the models folder.
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// A model as edits refer to it (`MaskShape::Ai::model`), and the files it needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    /// Stable id stored in edits: never reuse one for different weights.
    pub id: &'static str,
    pub label: &'static str,
    pub files: &'static [ModelFile],
    pub license: &'static str,
}

impl Model {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// Subject: silueta, a pruned U²-Net (rembg). Apache-2.0.
pub const SUBJECT: Model = Model {
    id: "subject@1",
    label: "Subject",
    license: "Apache-2.0 (U²-Net; rembg's silueta)",
    files: &[ModelFile {
        name: "silueta.onnx",
        url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/silueta.onnx",
        size: 44_173_029,
        sha256: "75da6c8d2f8096ec743d071951be73b4a8bc7b3e51d9a6625d63644f90ffeedb",
    }],
};

/// Sky and people: SegFormer-B0 on ADE20K. NVIDIA's licence: non-commercial use only.
pub const SCENE: Model = Model {
    id: "scene@1",
    label: "Sky & People",
    license: "NVIDIA SegFormer licence (non-commercial)",
    files: &[ModelFile {
        name: "segformer-b0-ade.onnx",
        url: "https://huggingface.co/Xenova/segformer-b0-finetuned-ade-512-512/resolve/d3e5499fa8701ff0453ca940a8dfeae39b2f1504/onnx/model.onnx",
        size: 15_335_446,
        sha256: "3e5c18a4be395f16646438d54c42377ddc202edfa33d5eced0c9506de75c44c2",
    }],
};

/// Select object: MobileSAM (encoder + single-mask decoder). Apache-2.0.
pub const OBJECT: Model = Model {
    id: "sam@1",
    label: "Select Object",
    license: "Apache-2.0 (MobileSAM)",
    files: &[
        ModelFile {
            name: "mobilesam-encoder.onnx",
            url: "https://huggingface.co/Acly/MobileSAM/resolve/0d3b403339b4674a82493d5e97964dd78089ddc8/mobile_sam_image_encoder.onnx",
            size: 28_157_093,
            sha256: "580f5fb648ea1062c0aabc26217aed56921985f03f0cbbd852bba81d760cc749",
        },
        ModelFile {
            name: "mobilesam-decoder.onnx",
            url: "https://huggingface.co/Acly/MobileSAM/resolve/0d3b403339b4674a82493d5e97964dd78089ddc8/sam_mask_decoder_single.onnx",
            size: 16_501_323,
            sha256: "93915fc7c993ab9d59ab8c9ccd3bce37f7509c81ab4150a74abd4d2abbd8570d",
        },
    ],
};

pub const ALL: &[Model] = &[SUBJECT, SCENE, OBJECT];

pub fn by_id(id: &str) -> Option<&'static Model> {
    ALL.iter().find(|m| m.id == id)
}
