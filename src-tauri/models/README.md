# Models

All three run on the CPU through tract (`src/segment.rs`).

- `u2netp.onnx` finds a photo's main subject. It is U²-Netp: Xuebin Qin et al., [U²-Net: Going Deeper with Nested U-Structure for Salient Object Detection](https://github.com/xuebinqin/U-2-Net), released under the Apache License 2.0 (`U2NET-LICENSE`). These are the authors' weights, as exported to ONNX by [rembg](https://github.com/danielgatis/rembg) (MIT).
  sha256 `309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8`
- `u2netp_sky.onnx` finds the sky. It is U²-Netp too, with xiongzhu666's sky weights, `skysegsmall_sim-opt-fp16` from [Sky-Segmentation-and-Post-processing](https://github.com/xiongzhu666/Sky-Segmentation-and-Post-processing) (MIT, `SKYSEG-LICENSE`), published for ncnn. `tools/ncnn2onnx.py` turned it into ONNX; its output matches ncnn's to within 3e-6.
  sha256 `4482041d07e6ca1758abdd224d0f3129d746107e6a56f52f62e3088d2ec2c727` (from `.param` `7af35231…`, `.bin` `ff1e18f7…`)

  ```sh
  python tools/ncnn2onnx.py skysegsmall_sim-opt-fp16.param skysegsmall_sim-opt-fp16.bin u2netp_sky.onnx   # needs numpy and onnx
  ```
- `efficient_sam_vitt_encoder.onnx` and `efficient_sam_vitt_decoder.onnx` find the object inside a loop drawn on the photo, given the loop's box. They are EfficientSAM-Ti, from Yunyang Xiong et al., [EfficientSAM](https://github.com/yformer/EfficientSAM), released under the Apache License 2.0 (`EFFICIENTSAM-LICENSE`), as exported to ONNX by its authors.
  sha256 encoder `84ed466ffcc5c1f8d08409bc34a23bb364ab2c15e402cb12d4335a42be0e0951`, decoder `a62f8fa5ea080447c0689418d69e58f1e83e0b7adf9c142e2bd9bcc8045c0b11`

Changing a model, or how its answer is refined, means changing its tag in `segment.rs`, so mattes cached for the old one are found again.
