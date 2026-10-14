"""Run Blender's native mesh algorithms on a disposable mesh, never a user's scene."""
from __future__ import annotations

import copy
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile
import time

import numpy as np

from . import edges, groups, topo
from .blender_mesh_catalog import OPERATORS
from .blender_setup import find_blender
from .geometry import Geometry
from .nodes._build import make as build

MAX_POINTS = 250000
MAX_CORNERS = 1000000
WORKER = Path(__file__).resolve().parents[1]/'blender_addon'/'minihou_link'/'native_mesh_worker.py'


def runtime_key(params):
    exe = find_blender()
    if not exe:
        return 'missing-blender'
    paths = [Path(exe), WORKER, Path(__file__).with_name('blender_mesh_catalog.py')]
    parts = []
    for path in paths:
        stat = path.stat()
        parts.append(f'{path}:{stat.st_size}:{stat.st_mtime_ns}')
    return '|'.join(parts)



def encode(geo, domain, group):
    geo.validate()
    if geo.npoints > MAX_POINTS or geo.nvertices > MAX_CORNERS:
        raise ValueError('Native Blender modelling input exceeds 250,000 points / 1,000,000 corners')
    if not np.isfinite(geo.P).all():
        raise ValueError('Native Blender modelling requires finite positions')
    t = geo.topology
    primitives = [t.vertex_point[t.prim_offsets[i]:t.prim_offsets[i+1]].tolist() for i in range(geo.nprims)]
    closed = t.closed_mask().tolist()
    # Blender has no independent corner/face data for loose edges. Refuse loss on import.
    if not all(closed) and (any(not name.startswith(edges.PREFIX) and name != edges.CREASE for name in geo.vertex) or len(geo.prim)):
        raise ValueError('Native Blender modelling cannot import face/corner attributes on open curves; convert or remove them first')
    attributes = []
    for source in ('point', 'vertex', 'prim'):
        for name, a in getattr(geo, source).items():
            if name in ('P', 'N') or source == 'vertex' and (name.startswith(edges.PREFIX) or name == edges.CREASE):
                continue
            if not np.isfinite(a).all():
                raise ValueError(f'Native Blender modelling requires finite attribute {name!r}')
            attributes.append({'domain': source, 'name': name, 'dtype': str(a.dtype),
                               'components': a.shape[1] if a.ndim == 2 else 1, 'vector': a.ndim == 2,
                               'values': a.tolist()})
    derived = topo.edges(geo)
    edge_attrs = []
    for name in geo.vertex:
        if name.startswith(edges.PREFIX) or name == edges.CREASE:
            edge_attrs.append({'name': name, 'values': edges.values(geo, name).tolist(),
                               'dtype': str(geo.vertex[name].dtype)})
    if domain == 'edge':
        selected = np.flatnonzero(edges.mask(geo, group)).tolist()
    else:
        mask = groups.mask(geo, 'point' if domain == 'point' else 'prim', group)
        selected = (np.arange(geo.npoints if domain == 'point' else geo.nprims) if mask is None else np.flatnonzero(mask)).tolist()
    return {'points': geo.P.tolist(), 'primitives': primitives, 'closed': closed,
            'edges': derived.points.tolist(), 'attributes': attributes, 'edge_attributes': edge_attrs,
            'domain': domain, 'selected': selected}


def decode(result, detail):
    P = np.asarray(result['points'], np.float32).reshape(-1, 3)
    primitives = result['primitives']
    sizes = np.asarray([len(p) for p in primitives], np.int32)
    vp = np.asarray([v for p in primitives for v in p], np.int32)
    out = build(P, vp, np.r_[0, np.cumsum(sizes)].astype(np.int32), closed=np.asarray(result['closed'], bool))
    for spec in result['attributes']:
        a = np.asarray(spec['values'], dtype=spec['dtype'])
        n = getattr(out, spec['domain']).size
        a = a.reshape((n, spec['components']) if spec['vector'] else (n,))
        getattr(out, spec['domain'])[spec['name']] = a
    if result.get('edge_attributes'):
        e = topo.edges(out)
        index = {tuple(pair): i for i, pair in enumerate(result['edges'])}
        for spec in result['edge_attributes']:
            values = np.asarray([spec['values'][index[tuple(pair)]] for pair in e.points], dtype=spec['dtype'])
            out = edges.write(out, spec['name'], values)
    out.detail = copy.deepcopy(detail)
    out.detail['native_blender_version'] = result['version']
    out.validate()
    return out


def execute(geo, operation, domain='face', group='', options=None, result_group='native_selection'):
    if operation not in OPERATORS:
        raise ValueError(f'Unsupported native Blender mesh operator: {operation}')
    if domain not in ('point', 'edge', 'face'):
        raise ValueError('Native Blender selection domain must be point, edge or face')
    if options is not None and not isinstance(options, dict):
        raise ValueError('Native Blender operator options must be a JSON object')
    if result_group and not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', result_group):
        raise ValueError('Native result group must use letters, digits and underscores')
    exe = find_blender()
    if not exe:
        raise ValueError('Native mesh tool requires Blender on PATH or BLENDER_EXE; existing MiniHou nodes do not require Blender')
    request = encode(geo, domain, group)
    request.update(operation=operation, options=options or {}, result_group=result_group)
    worker = WORKER
    with tempfile.TemporaryDirectory(prefix='minihou-native-mesh-') as tmp:
        src, dst = Path(tmp)/'input.json', Path(tmp)/'output.json'
        src.write_text(json.dumps(request, allow_nan=False), encoding='utf-8')
        env = dict(os.environ, BLENDER_USER_CONFIG=str(Path(tmp)/'config'))
        log = Path(tmp)/'worker.log'
        with log.open('w', encoding='utf-8') as stream:
            try:
                proc = subprocess.Popen([exe, '--background', '--factory-startup', '--disable-autoexec', '--threads', '1',
                                         '--python-exit-code', '1', '--python', str(worker), '--', str(src), str(dst)],
                                        stdout=stream, stderr=subprocess.STDOUT, env=env)
            except OSError as e:
                raise ValueError(f'Cannot launch Blender: {e}') from e
            try:
                deadline = time.monotonic()+90
                while proc.poll() is None:
                    if time.monotonic() > deadline:
                        raise ValueError('Native Blender modelling exceeded its 90-second time limit')
                    time.sleep(.05)
            except BaseException:
                proc.kill(); proc.wait()
                raise
        if proc.returncode or not dst.exists():
            tail = log.read_text(encoding='utf-8', errors='replace')[-2500:].strip()
            raise ValueError(f'Blender mesh.{operation} failed: {tail}')
        result = json.loads(dst.read_text(encoding='utf-8'))
        if len(result['points']) > MAX_POINTS or sum(map(len, result['primitives'])) > MAX_CORNERS:
            raise ValueError('Native Blender modelling output exceeds its geometry limit')
        return decode(result, geo.detail)
