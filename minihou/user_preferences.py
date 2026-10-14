"""Explicit per-user engine preferences, never loaded from graph files."""
import json
import os
import math
import tempfile
import threading
from pathlib import Path

COPY_LIMIT_DEFAULT = 256
COPY_LIMIT_MIN = 16
COPY_LIMIT_MAX = 16384
_LOCK = threading.RLock()


def defaults():
    return {'python_nodes': False, 'copy_memory_limit_mib': COPY_LIMIT_DEFAULT,
            'length_unit': 'mm', 'unit_scale': 1.0, 'isolated_knockback': .75,
            'proxy': {'level': 1.0, 'auto': False, 'link_tris': 0}}


def _display_preference(key, value):
    if key == 'length_unit':
        if value not in ('mm', 'cm', 'm', 'km', 'in', 'ft'):
            raise ValueError('unsupported length unit')
        return value
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value):
        raise ValueError(f'{key} must be finite')
    if key == 'unit_scale' and not 1e-9 <= value <= 1e9:
        raise ValueError('unit_scale must be between 1e-9 and 1e9')
    if key == 'isolated_knockback' and not 0 <= value <= 1:
        raise ValueError('isolated_knockback must be between 0 and 1')
    return float(value)


def _copy_limit(value):
    if isinstance(value, bool) or not isinstance(value, int) or not COPY_LIMIT_MIN <= value <= COPY_LIMIT_MAX:
        raise ValueError(f'copy_memory_limit_mib must be a whole number between {COPY_LIMIT_MIN} and {COPY_LIMIT_MAX}')
    return value


def _proxy(value):
    if not isinstance(value, dict):
        raise ValueError('proxy must be an object')
    out = {}
    if 'level' in value:
        level = value['level']
        if isinstance(level, bool) or not isinstance(level, (int, float)) or not math.isfinite(level):
            raise ValueError('proxy level must be a finite number')
        out['level'] = min(1.0, max(.001, float(level)))
    if 'auto' in value:
        if not isinstance(value['auto'], bool):
            raise ValueError('proxy auto must be a boolean')
        out['auto'] = value['auto']
    if 'link_tris' in value:
        cap = value['link_tris']
        if isinstance(cap, bool) or not isinstance(cap, int) or cap < 0:
            raise ValueError('proxy link_tris must be a nonnegative whole number')
        out['link_tris'] = cap
    return out


def path():
    return Path(os.environ.get("MINIHOU_PREFERENCES_FILE") or
                Path(os.environ.get("LOCALAPPDATA", Path.home() / ".config")) / "MiniHou" / "preferences.json")


def read():
    out = defaults()
    try:
        data = json.loads(path().read_text(encoding="utf-8"))
        out['python_nodes'] = data.get('python_nodes') is True
        try:
            out['copy_memory_limit_mib'] = _copy_limit(data.get('copy_memory_limit_mib', COPY_LIMIT_DEFAULT))
        except ValueError:
            pass
        for key in ('length_unit', 'unit_scale', 'isolated_knockback'):
            if key in data:
                try:
                    out[key] = _display_preference(key, data[key])
                except ValueError:
                    pass
        saved_proxy = data.get('proxy', {})
        if isinstance(saved_proxy, dict):
            for key in out['proxy']:
                if key in saved_proxy:
                    try:
                        out['proxy'].update(_proxy({key: saved_proxy[key]}))
                    except ValueError:
                        pass
    except (OSError, ValueError, AttributeError):
        pass
    return out


def describe():
    out = read()
    out['limits'] = {'copy_memory_limit_mib': {'min': COPY_LIMIT_MIN, 'max': COPY_LIMIT_MAX,
                                             'default': COPY_LIMIT_DEFAULT}}
    return out


def update(values):
    """Validate a partial update and atomically preserve all other known settings."""
    if not isinstance(values, dict) or not values or set(values)-{'python_nodes', 'copy_memory_limit_mib', 'proxy', 'length_unit', 'unit_scale', 'isolated_knockback'}:
        raise ValueError('preferences requires python_nodes, copy_memory_limit_mib or proxy')
    clean = {}
    for key in ('length_unit', 'unit_scale', 'isolated_knockback'):
        if key in values:
            clean[key] = _display_preference(key, values[key])
    if 'python_nodes' in values:
        if not isinstance(values['python_nodes'], bool):
            raise ValueError('python_nodes must be a boolean')
        clean['python_nodes'] = values['python_nodes']
    if 'copy_memory_limit_mib' in values:
        clean['copy_memory_limit_mib'] = _copy_limit(values['copy_memory_limit_mib'])
    if 'proxy' in values:
        clean['proxy'] = _proxy(values['proxy'])
    with _LOCK:
        out = read()
        proxy_update = clean.pop('proxy', {})
        out.update(clean)
        out['proxy'].update(proxy_update)
        target = path()
        target.parent.mkdir(parents=True, exist_ok=True)
        descriptor, temporary = tempfile.mkstemp(prefix='preferences-', suffix='.tmp', dir=target.parent)
        try:
            with os.fdopen(descriptor, 'w', encoding='utf-8') as stream:
                json.dump(out, stream)
            os.replace(temporary, target)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)
        return out


def copy_limit_bytes():
    return read()['copy_memory_limit_mib'] * 1024**2


def set_python(value):
    return update({'python_nodes': value})
