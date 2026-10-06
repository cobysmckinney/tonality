# Models

All of them run on the CPU through tract (`src/segment.rs`).

- `isnet_general_use.f16.onnx` finds a photo's main subject. It is IS-Net: Xuebin Qin et al., [Highly Accurate Dichotomous Image Segmentation](https://github.com/xuebinqin/DIS), released under the Apache License 2.0 (`U2NET-LICENSE` is the same licence, from the same authors). These are the authors' `isnet-general-use` weights, as exported to ONNX by [rembg](https://github.com/danielgatis/rembg) (MIT, sha256 `60920e99…`), with the weights then stored as 16-bit floats by `tools/half_weights.py` to halve the file; it still computes in 32 bits, and its mattes match the original's to within 2e-5.
  sha256 `d22692a589b287330a6d8edfb85ff66712251ff76973c1bc3fa63ec533d00d05`

  ```sh
  python tools/half_weights.py isnet-general-use.onnx isnet_general_use.f16.onnx   # needs numpy and onnx
  ```
- `u2netp_sky.onnx` finds the sky. It is U²-Netp (`U2NET-LICENSE`), with xiongzhu666's sky weights, `skysegsmall_sim-opt-fp16` from [Sky-Segmentation-and-Post-processing](https://github.com/xiongzhu666/Sky-Segmentation-and-Post-processing) (MIT, `SKYSEG-LICENSE`), published for ncnn. `tools/ncnn2onnx.py` turned it into ONNX; its output matches ncnn's to within 3e-6.
  sha256 `4482041d07e6ca1758abdd224d0f3129d746107e6a56f52f62e3088d2ec2c727` (from `.param` `7af35231…`, `.bin` `ff1e18f7…`)

  ```sh
  python tools/ncnn2onnx.py skysegsmall_sim-opt-fp16.param skysegsmall_sim-opt-fp16.bin u2netp_sky.onnx   # needs numpy and onnx
  ```
- `ade20k_mobilenetv2.onnx` checks the sky model: a scene parser that says how likely each part of the photo is to be each of ADE20K's 150 kinds of thing, sky among them. It sees snow on a mountain as mountain, and sky through leaves as sky, where the sky model gets both wrong. It is MIT CSAIL's MobileNetV2dilated-C1 from [semantic-segmentation-pytorch](https://github.com/CSAILVision/semantic-segmentation-pytorch) (BSD 3-Clause, `CSAIL-SEMSEG-LICENSE`), its `ade20k-mobilenetv2dilated-c1_deepsup` weights (encoder sha256 `f9271676…`, decoder `23acb59a…`) exported to ONNX by `tools/export_ade20k.py`.
  sha256 `711ca4a4f451c62565235704aa94da97d2d24da6f52b5415e2cd7b7cddf1b9f9`

  ```sh
  python tools/export_ade20k.py <checkout> encoder_epoch_20.pth decoder_epoch_20.pth ade20k_mobilenetv2.onnx   # needs torch
  ```
- `efficient_sam_vitt_encoder.onnx` and `efficient_sam_vitt_decoder.onnx` find the object inside a loop drawn on the photo, given the loop's box. They are EfficientSAM-Ti, from Yunyang Xiong et al., [EfficientSAM](https://github.com/yformer/EfficientSAM), released under the Apache License 2.0 (`EFFICIENTSAM-LICENSE`), as exported to ONNX by its authors.
  sha256 encoder `84ed466ffcc5c1f8d08409bc34a23bb364ab2c15e402cb12d4335a42be0e0951`, decoder `a62f8fa5ea080447c0689418d69e58f1e83e0b7adf9c142e2bd9bcc8045c0b11`

Changing a model, or how its answer is refined, means changing its tag in `segment.rs`, so mattes cached for the old one are found again.
