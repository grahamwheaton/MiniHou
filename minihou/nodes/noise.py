import numpy as np

from ..sdk import Enum, Float, Int, Str, Vec3, node
from ._util import fbm3


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
])
def noise(geo, attrib, mode, amplitude, frequency, offset, octaves, lacunarity, gain, seed):
    """Fractal Perlin noise sampled at P * frequency + offset.

    Output range is roughly [-1, 1] before 'amplitude'. For ``attrib == "P"`` the result is
    added to P. For other attributes it is written as a float attribute in scalar mode
    (value * amplitude.x) or a vec3 in vector mode.
    """
    if not attrib:
        raise ValueError("'attrib' must not be empty")
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
    return out
