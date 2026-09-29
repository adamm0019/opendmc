"""A minimal KTX2 writer for baked lighting (ADR-013): one mip level, RGBA16F,
zstd supercompression, as a 2D image, a cubemap (six faces) or a 3D texture.

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


def _write(path, raw, width, height, depth, faces, level):
    import zstandard

    data = zstandard.ZstdCompressor(level=level).compress(raw)
    dfd = _dfd_rgba16f()
    header = 12 + 9 * 4 + 4 * 4 + 2 * 8
    dfd_offset = header + 3 * 8
    data_offset = dfd_offset + len(dfd)
    out = bytearray(IDENTIFIER)
    out += struct.pack("<9I", VK_FORMAT_R16G16B16A16_SFLOAT, 2, width, height, depth, 0, faces, 1, ZSTANDARD)
    out += struct.pack("<4I2Q", dfd_offset, len(dfd), 0, 0, 0, 0)
    out += struct.pack("<3Q", data_offset, len(data), len(raw))
    out += dfd
    assert len(out) == data_offset
    out += data
    with open(path, "wb") as f:
        f.write(out)


# Half floats top out at 65504; anything above (a light seen directly) would
# be stored as infinity, and filtering turns that into NaN.
MAX_VALUE = 60000.0


def _rgba16f(a):
    a = np.nan_to_num(np.clip(a[..., :4], 0.0, MAX_VALUE), nan=0.0)
    return np.ascontiguousarray(a, dtype=np.float16).tobytes()


def write(path, faces, level=10):
    """Write `faces` (arrays of shape (h, w, 4), top row first): one face for
    a 2D image, six (+X, -X, +Y, -Y, +Z, -Z) for a cubemap."""
    assert len(faces) in (1, 6)
    h, w = faces[0].shape[:2]
    _write(path, b"".join(_rgba16f(f) for f in faces), w, h, 0, len(faces), level)


def write_3d(path, volume, level=10):
    """Write a 3D texture from an array of shape (depth, height, width, 4)."""
    d, h, w = volume.shape[:3]
    _write(path, _rgba16f(volume), w, h, d, 1, level)
