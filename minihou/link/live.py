"""Connection-owned live modifier documents; Blender data stays in the engine process."""
from dataclasses import dataclass, field
import hashlib
from .diff import DiffState


def input_token(geometry):
    geometry.validate()
    digest = hashlib.blake2b(digest_size=16)
    _, arrays = DiffState().build(geometry, 0)
    for key, array in sorted(arrays.items()):
        digest.update(key.encode())
        digest.update(str((array.dtype.str, array.shape)).encode())
        digest.update(array.tobytes())
    return digest.hexdigest()


@dataclass
class LiveModifier:
    instance_id: str
    label: str
    graph: object
    path: object
    geometry: object = None
    frame: float = 1.0
    fps: float = 24.0
    token: str = ""
    sent_params: dict = field(default_factory=dict)
    connected: bool = True
