"""Converts the U2NetP sky model from ncnn's param/bin to ONNX (only the layer types it uses)."""
import struct, sys, numpy as np, onnx
from onnx import helper, TensorProto, numpy_helper

param, binf, out = sys.argv[1:4]
lines = open(param).read().split("\n")[2:]
data = open(binf, "rb").read(); pos = 0

def take_weights(count):
    global pos
    flag = struct.unpack_from("<I", data, pos)[0]; pos += 4
    if flag == 0x01306B47:
        w = np.frombuffer(data, np.float16, count, pos).astype(np.float32)
        pos += (count * 2 + 3) // 4 * 4
    elif flag == 0:
        w = np.frombuffer(data, np.float32, count, pos).copy(); pos += count * 4
    else:
        raise SystemExit(f"unsupported weight flag {flag:#x}")
    return w

def take_raw(count):
    global pos
    w = np.frombuffer(data, np.float32, count, pos).copy(); pos += count * 4
    return w

nodes, inits = [], []
alias = {}  # split outputs -> the blob they copy
name = lambda b: alias.get(b, b)
channels = {}
for line in lines:
    parts = line.split()
    if not parts: continue
    kind, layer, ni, no = parts[0], parts[1], int(parts[2]), int(parts[3])
    ins, outs = parts[4:4 + ni], parts[4 + ni:4 + ni + no]
    kv = dict(p.split("=") for p in parts[4 + ni + no:])
    if kind == "Input":
        channels[outs[0]] = 3
    elif kind == "Split":
        for o in outs: alias[o] = name(ins[0])
    elif kind == "Convolution":
        cout, k = int(kv["0"]), int(kv["1"]); dil = int(kv.get("2", 1)); pad = int(kv.get("4", 0))
        size = int(kv["6"]); cin = size // (cout * k * k)
        w = take_weights(size).reshape(cout, cin, k, k)
        inputs = [name(ins[0]), layer + "_w"]
        inits.append(numpy_helper.from_array(w, layer + "_w"))
        if kv.get("5") == "1":
            inits.append(numpy_helper.from_array(take_raw(cout), layer + "_b")); inputs.append(layer + "_b")
        act = kv.get("9", "0")
        conv_out = outs[0] if act == "0" else layer + "_pre"
        channels[outs[0]] = cout
        nodes.append(helper.make_node("Conv", inputs, [conv_out], kernel_shape=[k, k], dilations=[dil, dil], pads=[pad] * 4, strides=[1, 1]))
        if act == "1": nodes.append(helper.make_node("Relu", [conv_out], outs))
        elif act == "4": nodes.append(helper.make_node("Sigmoid", [conv_out], outs))
        elif act != "0": raise SystemExit(f"activation {act}")
    elif kind == "Pooling":
        assert kv.get("0", "0") == "0" and kv["1"] == "2" and kv["2"] == "2"
        channels[outs[0]] = channels[name(ins[0])]
        nodes.append(helper.make_node("MaxPool", [name(ins[0])], outs, kernel_shape=[2, 2], strides=[2, 2], ceil_mode=1))
    elif kind == "Interp":
        assert kv["0"] == "2"
        h, w = int(kv["3"]), int(kv["4"])
        c = channels[name(ins[0])]; channels[outs[0]] = c
        inits.append(numpy_helper.from_array(np.array([1, c, h, w], np.int64), layer + "_sizes"))
        nodes.append(helper.make_node("Resize", [name(ins[0]), "", "", layer + "_sizes"], outs, mode="linear", coordinate_transformation_mode="half_pixel"))
    elif kind == "Concat":
        channels[outs[0]] = sum(channels[name(i)] for i in ins)
        nodes.append(helper.make_node("Concat", [name(i) for i in ins], outs, axis=1))
    elif kind == "BinaryOp":
        assert kv.get("0", "0") == "0"
        channels[outs[0]] = channels[name(ins[0])]
        nodes.append(helper.make_node("Add", [name(i) for i in ins], outs))
    elif kind == "Sigmoid":
        channels[outs[0]] = channels[name(ins[0])]
        nodes.append(helper.make_node("Sigmoid", [name(ins[0])], outs))
    else:
        raise SystemExit(f"layer {kind}")
assert pos == len(data), (pos, len(data))
# Only the fused map is wanted; the side outputs' sigmoids are left unused.
graph = helper.make_graph(nodes, "u2netp_sky", [helper.make_tensor_value_info("input.1", TensorProto.FLOAT, [1, 3, 384, 384])],
                          [helper.make_tensor_value_info("1959", TensorProto.FLOAT, [1, 1, 384, 384])], inits)
model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 13)])
model.ir_version = 8
onnx.checker.check_model(model)
onnx.save(model, out)
print("ok", len(nodes), "nodes", sum(i.ByteSize() for i in inits) // 1024, "KiB")
