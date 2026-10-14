import numpy as np

from .. import accel, groups
from ..sdk import Enum, Float, Int, Str, Vec3, node
from ._util import fbm3


def _noise_fused(geo, attrib, mode, amplitude, frequency, offset, octaves, lacunarity, gain, seed):
    """One-pass Numba version for the vec3 cases (P displacement, or a vector attribute). Returns None to
    use the numpy path (acceleration off/failed, or the scalar-attribute case which is not hot)."""
    from .. import rust
    vector = mode == "vector"
    add = attrib == "P"
    if rust.enabled():
        if not (add or vector):
            return None                                   # the numpy path, with Rust's fbm3
        res = geo.copy()                                  # one parallel pass, bit for bit the numpy path's result
        res.point[attrib] = rust.noise_vec(geo.P, frequency, offset, amplitude, seed, octaves, lacunarity, gain, vector, add)
        return res
    k = accel.kernels()
    if k is None or not (add or vector):
        return None
    P = np.ascontiguousarray(geo.P)
    out = np.empty_like(P)
    r = accel.call(k.noise_vec, P, float(frequency), np.asarray(offset, np.float32), np.asarray(amplitude, np.float32),
                   int(seed) & 0xFFFFFFFF, int(octaves), float(lacunarity), float(gain), vector, add, out)
    if r is accel.FAILED:
        return None
    res = geo.copy()
    res.point[attrib] = out
    return res


@node("noise", inputs=["geo"], category="modify", params=[
    Str("attrib", "P", doc="Point attribute to write. 'P' displaces; any other name is set (not added)."),
    Enum("mode", ["scalar", "vector"], "scalar",
         doc="scalar: one noise value times 'amplitude'. vector: three decorrelated values times 'amplitude'."),
    Vec3("amplitude", (0.0, 0.0, 1.0)),
    Float("frequency", 1.0, min=0.0, soft_max=20.0),
    Vec3("offset", (0.0, 0.0, 0.0), doc="Added to the sample position after frequency scaling"),
    Int("octaves", 1, min=1, max=12),
    Float("lacunarity", 2.0, min=1.0, soft_max=4.0),
    Float("gain", 0.5, min=0.0, max=1.0),
    Int("seed", 0),
    Str("group", "", doc="Only change these points: group names, ^exclusions, numbers/ranges or '@P.z > 0'. Empty: all"),
])
def noise(geo, attrib, mode, amplitude, frequency, offset, octaves, lacunarity, gain, seed, group=""):
    """Fractal Perlin noise sampled at P * frequency + offset.

    Output range is roughly [-1, 1] before 'amplitude'. For ``attrib == "P"`` the result is
    added to P. For other attributes it is written as a float attribute in scalar mode
    (value * amplitude.x) or a vec3 in vector mode.
    """
    if not attrib:
        raise ValueError("'attrib' must not be empty")
    fused = _noise_fused(geo, attrib, mode, amplitude, frequency, offset, octaves, lacunarity, gain, seed)
    if fused is not None:
        return groups.restrict(geo, fused, "point", group)
    pos = geo.P * np.float32(frequency) + np.asarray(offset, np.float32)
    amp = np.asarray(amplitude, np.float32)
    if mode == "scalar":
        n = fbm3(pos, seed, octaves, lacunarity, gain)
        vec = n[:, None] * amp[None, :]
        scalar = n * amp[0]
    else:
        n3 = np.stack([fbm3(pos, seed + 1013 * c, octaves, lacunarity, gain) for c in range(3)], axis=1)
        vec = n3 * amp[None, :]
        scalar = None
    out = geo.copy()
    if attrib == "P":
        out.point["P"] = geo.P + vec
    elif mode == "scalar":
        out.point[attrib] = scalar
    else:
        out.point[attrib] = vec
    return groups.restrict(geo, out, "point", group)
