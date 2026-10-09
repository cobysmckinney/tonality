# Models

All of them run on the CPU through tract (`src/segment.rs`).

- `isnet_general_use.f16.onnx` finds a photo's main subject. It is IS-Net: Xuebin Qin et al., [Highly Accurate Dichotomous Image Segmentation](https://github.com/xuebinqin/DIS), whose code is under the Apache License 2.0 (`U2NET-LICENSE` is the same licence, from the same authors; the weights have no licence of their own, see [Training data](#training-data)). These are the authors' `isnet-general-use` weights, as exported to ONNX by [rembg](https://github.com/danielgatis/rembg) (MIT, `REMBG-LICENSE`, sha256 `60920e99…`), with the weights then stored as 16-bit floats by `tools/half_weights.py` to halve the file; it still computes in 32 bits, and its mattes match the original's to within 2e-5.
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
- `efficient_sam_vitt_encoder.onnx` and `efficient_sam_vitt_decoder.onnx` find the object inside a loop drawn on the photo, given the loop's box. They are EfficientSAM-Ti, from Yunyang Xiong et al., [EfficientSAM](https://github.com/yformer/EfficientSAM), released under the Apache License 2.0 (`EFFICIENTSAM-LICENSE`), as exported to ONNX by Kentaro Wada and published by its authors.
  sha256 encoder `84ed466ffcc5c1f8d08409bc34a23bb364ab2c15e402cb12d4335a42be0e0951`, decoder `a62f8fa5ea080447c0689418d69e58f1e83e0b7adf9c142e2bd9bcc8045c0b11`

Changing a model, or how its answer is refined, means changing its tag in `segment.rs`, so mattes cached for the old one are found again.

Each licence file here is shown, with the model it belongs to, under Third-party notices in the app's About screen. `bun run notices` writes that list.

## Training data

The licences above cover the code and, where the authors say so, the weights. The pictures the models learned from have terms of their own, and all of them are for non-commercial research. None of those terms mention models trained on the pictures, and none of the model authors say whether the data's terms carry over to their weights. What the sources say, as of October 2026:

- **IS-Net (subject).** The `isnet-general-use` weights were trained on data the authors haven't described; the [DIS README](https://github.com/xuebinqin/DIS/blob/main/README.md) says only that it is "NOT DIS V2.0", and DIS5K is its published dataset. The README limits the Apache License to "Our code and evaluation metric"; the weights have no licence stated at all. The [DIS5K terms of use](https://github.com/xuebinqin/DIS/blob/main/DIS5K-Dataset-Terms-of-Use.pdf) say the images were "collected from Flickr", that the dataset "is available for non-commercial use in research or educational purpose", and that "commercial use of this dataset is prohibited even after copying, editing, processing or any operations of this database". [Issue #150](https://github.com/xuebinqin/DIS/issues/150) asks the author whether the weights may be used commercially; it has no answer yet. rembg, which exported the ONNX file, says in its [README](https://github.com/danielgatis/rembg/blob/main/README.md) that "Model weights carry their own licenses, independent of rembg's MIT license". This is the least clear of the models.
- **U²-Netp sky weights.** The author [says](https://github.com/xiongzhu666/Sky-Segmentation-and-Post-processing/issues/1) the small model was trained on ADE20K. The repository is MIT, with nothing separate for the weights.
- **ADE20K scene parser.** Trained on ADE20K, starting from an encoder trained on ImageNet ([mobilenet.py](https://github.com/CSAILVision/semantic-segmentation-pytorch/blob/master/mit_semseg/models/mobilenet.py)). The [ADE20K terms](https://ade20k.csail.mit.edu/terms/) say "MIT, CSAIL does not own the copyright of the images" and "Researcher shall use the Database only for non-commercial research and educational purposes", while its annotations and software are under BSD-3. [ImageNet's terms](https://image-net.org/download.php) have the same non-commercial line. The repository is BSD-3, with nothing separate for the weights.
- **EfficientSAM.** Pretrained on ImageNet-1K, learning from SAM's image encoder, then trained on SA-1B ([paper](https://arxiv.org/abs/2312.00863)). Meta's [SA-1B page](https://ai.meta.com/datasets/segment-anything/) says "Research purposes only"; the SA-1B Dataset Research License ([a copy](https://huggingface.co/datasets/xiuqhou/SA-Det-100k/blob/main/LICENSE); Meta's own page needs a browser) allows use "for Research Purposes only", defined as "on a non-commercial basis", and forbids distributing the images or "derivative works thereof" for "any commercial or production purpose". Whether a trained model is such a derivative work, it doesn't say. The repository is Apache-2.0, with nothing separate for the weights. For comparison, Meta's own [SAM](https://github.com/facebookresearch/segment-anything), trained on the same data, says "The model is licensed under the Apache 2.0 license"; [a question](https://github.com/facebookresearch/segment-anything/issues/63) about which licence its weights are under has no answer from Meta.

None of this is legal advice, and the sources don't settle whether these terms reach an app that ships the weights. Anyone who wants to use the models commercially should ask their authors, as issue #150 does.
