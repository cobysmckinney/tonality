"""Stores an ONNX model's weights as 16-bit floats, to halve its size.

Each float weight becomes a half-precision constant followed by a Cast back
to 32 bits, so the model still computes in 32 bits; tract folds the casts
away when it loads the model.

    python half_weights.py model.onnx model.f16.onnx   # needs numpy and onnx
"""
import sys

import numpy as np
import onnx
from onnx import TensorProto, helper, numpy_helper

source, dest = sys.argv[1], sys.argv[2]
model = onnx.load(source)
graph = model.graph
casts, kept = [], []
for weight in graph.initializer:
    values = numpy_helper.to_array(weight)
    # Small tensors (shapes, scales) aren't worth it and may need full precision.
    if weight.data_type != TensorProto.FLOAT or values.size < 1024:
        kept.append(weight)
        continue
    half = numpy_helper.from_array(values.astype(np.float16), weight.name + "_f16")
    kept.append(half)
    casts.append(helper.make_node("Cast", [half.name], [weight.name], to=TensorProto.FLOAT, name=weight.name + "_cast"))
del graph.initializer[:]
graph.initializer.extend(kept)
nodes = casts + list(graph.node)
del graph.node[:]
graph.node.extend(nodes)
onnx.checker.check_model(model)
onnx.save(model, dest)
print(f"{len(casts)} weights halved")
