"""Exports MIT CSAIL's ADE20K scene parser (MobileNetV2dilated + C1) to ONNX.

A normalised 512 x 512 RGB picture in; each of ADE20K's 150 classes'
probabilities out, a 64 x 64 grid of them. Needs torch and a checkout of
https://github.com/CSAILVision/semantic-segmentation-pytorch, plus the
model's weights from
http://sceneparsing.csail.mit.edu/model/pytorch/ade20k-mobilenetv2dilated-c1_deepsup/

    python export_ade20k.py <checkout> encoder_epoch_20.pth decoder_epoch_20.pth ade20k_mobilenetv2.onnx
"""
import sys

import torch

checkout, encoder_weights, decoder_weights, dest = sys.argv[1:5]
sys.path.insert(0, checkout)
from mit_semseg.models import ModelBuilder  # noqa: E402

encoder = ModelBuilder.build_encoder(arch="mobilenetv2dilated", fc_dim=320, weights=encoder_weights)
decoder = ModelBuilder.build_decoder(arch="c1_deepsup", fc_dim=320, num_class=150, weights=decoder_weights, use_softmax=True)


class Parser(torch.nn.Module):
    """The encoder and the decoder's main head, without its upsampling to the input's size."""

    def __init__(self):
        super().__init__()
        self.encoder, self.decoder = encoder, decoder

    def forward(self, x):
        features = self.encoder(x, return_feature_maps=True)
        return torch.softmax(self.decoder.conv_last(self.decoder.cbr(features[-1])), dim=1)


side = 512
torch.onnx.export(
    Parser().eval(), torch.zeros(1, 3, side, side), dest, input_names=["image"], output_names=["classes"], opset_version=17, dynamo=False
)
