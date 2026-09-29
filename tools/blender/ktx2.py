"""A minimal KTX2 writer for baked lighting (ADR-013): one mip level, RGBA16F,
zstd supercompression, a 2D image or a cubemap (six faces).

Block-compressed formats (BC6H/BC7) come later from our own encoder; this is
enough for lightmaps and probes to load in the engine.
"""

import struct

import numpy as np

IDENTIFIER = bytes([0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A])
VK_FORMAT_R16G16B16A16_SFLOAT = 97
ZSTANDARD = 2


def _dfd_rgba16f():
    """A basic data format descriptor: linear RGBA, four signed 16-bit floats."""
    samples = b"".join(
        struct.pack("<HBB4BII", i * 16, 15, channel | 0xC0, 0, 0, 0, 0, 0xBF800000, 0x3F800000)
        for i, channel in enumerate((0, 1, 2, 15))
    )
    size = 24 + len(samples)
    block = struct.pack("<IHH4B4B8B", 0, 2, size,
                        1, 1, 1, 0,          # RGBSDA, BT.709, linear, straight alpha
                        0, 0, 0, 0,          # one texel per block
                        8, 0, 0, 0, 0, 0, 0, 0) + samples
    return struct.pack("<I", 4 + len(block)) + block


def write(path, faces, level=10):
    """Write `faces` (arrays of shape (h, w, 4), top row first): one face for
    a 2D image, six (+X, -X, +Y, -Y, +Z, -Z) for a cubemap."""
    import zstandard

    assert len(faces) in (1, 6)
    h, w = faces[0].shape[:2]
    raw = b"".join(np.ascontiguousarray(f[:, :, :4], dtype=np.float16).tobytes() for f in faces)
    data = zstandard.ZstdCompressor(level=level).compress(raw)
    dfd = _dfd_rgba16f()

    header = 12 + 9 * 4 + 4 * 4 + 2 * 8
    level_index = 3 * 8
    dfd_offset = header + level_index
    data_offset = dfd_offset + len(dfd)
    out = bytearray(IDENTIFIER)
    out += struct.pack("<9I", VK_FORMAT_R16G16B16A16_SFLOAT, 2, w, h, 0, 0, len(faces), 1, ZSTANDARD)
    out += struct.pack("<4I2Q", dfd_offset, len(dfd), 0, 0, 0, 0)
    out += struct.pack("<3Q", data_offset, len(data), len(raw))
    out += dfd
    assert len(out) == data_offset
    out += data
    with open(path, "wb") as f:
        f.write(out)
