"""Strict, external-process acceptance smoke for an accelerated MiniHou portable build.

Run with a Python containing Playwright and an installed Chromium, for example::

    python tools/morning_packaged_smoke.py --exe dist/MiniHou/MiniHou.exe --output build/packaged-smoke.json

No MiniHou source imports are used. Every cook, preview and UI edit runs in the
supplied executable. Missing acceleration, browser support or diagnostics fails.
The JSON report and adjacent scratch directory preserve evidence on failure.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import time
import traceback
import urllib.error
import urllib.request


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Api:
    def __init__(self, url, timeout):
        self.url, self.timeout = url.rstrip('/'), timeout

    def raw(self, path, body=None):
        request = urllib.request.Request(self.url + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={'Content-Type': 'application/json'})
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                return response.read()
        except urllib.error.HTTPError as error:
            raise RuntimeError(f'{path}: HTTP {error.code}: {error.read().decode(errors="replace")}') from error

    def json(self, path, body=None):
        result = json.loads(self.raw(path, body))
        require(result.get('ok') is not False, f'{path}: {result}')
        return result

    def preview(self, node, **fields):
        data = self.raw('/api/preview', dict(node=node, full=True, **fields))
        require(data[:4] == b'MHPV', f'preview failed: {data[:1000]!r}')
        size = struct.unpack_from('<I', data, 4)[0]
        head = json.loads(data[8:8 + size])
        arrays = {}
        for entry in head['arrays']:
            # The preview protocol currently uses 32-bit numeric arrays only.
            require(entry['dtype'] in ('float32', 'uint32', 'int32'), f'unsupported dtype: {entry}')
            start = 8 + size + entry['offset']
            arrays[entry['name']] = data[start:start + entry['count'] * 4]
        return head, arrays, len(data)

    def replace(self, graph):
        result = self.json('/api/edit', {'ops': [{'op': 'replace', 'graph': graph}], 'wait': True})
        require(result.get('status', {}).get('ok') is True, f'graph cook failed: {result}')
        return result


def group_graph():
    nodes = {'grid': {'type': 'grid', 'params': {'columns': 9, 'rows': 9, 'size_x': 8, 'size_y': 8}}}
    previous = 'grid'
    def add(name, kind, params):
        nonlocal previous
        nodes[name] = {'type': kind, 'inputs': {'geo': previous}, 'params': params}
        previous = name
    add('color', 'color', {'color': [1, 0, 0]})
    add('by_color', 'group_by_color', {'domain': 'vertices', 'name': 'red', 'color': [1, 0, 0], 'tolerance': 0})
    add('by_normal', 'group_by_normal', {'domain': 'primitives', 'name': 'facing', 'direction': [0, 0, 1], 'angle': 1})
    add('by_proximity', 'group_by_proximity', {'domain': 'edges', 'name': 'nearby', 'target_mode': 'position', 'position': [0, 0, 0], 'distance': 100})
    add('create', 'group_create', {'domain': 'points', 'name': 'seed', 'method': 'pattern', 'pattern': '0-2'})
    add('paint', 'group_paint', {'domain': 'edges', 'name': 'painted', 'selection': '0-2'})
    add('lasso', 'group_lasso', {'domain': 'vertices', 'name': 'lassoed', 'selection': '0-2'})
    add('expr', 'group_expression', {'domain': 'points', 'rules': [
        {'name': 'left', 'expression': '@P.x < 0'}, {'name': 'right', 'expression': '@P.x > 0'}]})
    add('combine', 'group_combine', {'rules': [
        {'domain': 'points', 'name': 'both', 'base': 'left', 'operations': [{'op': 'union', 'group': 'right'}]},
        {'domain': 'points', 'name': 'left_only', 'base': 'both', 'operations': [{'op': 'subtract', 'group': 'right'}]}]})
    add('rename', 'group_rename', {'rules': [
        {'domain': 'points', 'name': 'both', 'new_name': 'sides'},
        {'domain': 'points', 'name': 'left_only', 'new_name': 'selected'}]})
    add('promote', 'group_promote', {'rules': [
        {'from': 'points', 'to': 'primitives', 'name': 'seed', 'new_name': 'seed_faces', 'keep_original': True},
        {'from': 'primitives', 'to': 'edges', 'name': 'seed_faces', 'new_name': 'seed_edges', 'keep_original': True}]})
    return {'version': 1, 'nodes': nodes, 'display': previous}


def group_smoke(api):
    api.replace(group_graph())
    expected = [('vertex', 'red', 256), ('prim', 'facing', 64), ('edge', 'nearby', 144),
                ('point', 'seed', 3), ('edge', 'painted', 3), ('vertex', 'lassoed', 3),
                ('point', 'left', 36), ('point', 'right', 36), ('point', 'sides', 72), ('point', 'selected', 36)]
    checks = []
    for domain, name, count in expected:
        head, arrays, _ = api.preview('promote', mask={'name': name, 'domain': domain})
        require(head.get('mask', {}).get('members') == count, f'{domain} {name}: {head.get("mask")} expected {count}')
        require('mask' in arrays, f'missing mask array: {name}')
        checks.append({'domain': domain, 'group': name, 'members': count})
    for domain, name in [('prim', 'seed_faces'), ('edge', 'seed_edges')]:
        head, _, _ = api.preview('promote', mask={'name': name, 'domain': domain})
        require(head.get('mask', {}).get('members', 0) > 0, f'promotion missing {name}')
        checks.append({'domain': domain, 'group': name, 'members': head['mask']['members']})
    before, source, _ = api.preview('grid')
    after, result, _ = api.preview('promote')
    require(before['topology'] == after['topology'], 'group operations changed topology')
    require(source['P'] == result['P'], 'group operations changed point positions')
    return {'membership': checks, 'topology_preserved': True, 'positions_preserved': True}


def acceleration(api):
    state = api.json('/api/state')
    info = state.get('engine', {}).get('acceleration', {})
    require(info.get('available') and info.get('enabled'), f'Numba must be available and enabled: {info}')
    require(info.get('status', '').lower().startswith('numba '), f'Numba backend missing: {info}')
    require(isinstance(info.get('successful_calls'), dict), f'kernel execution diagnostics missing: {info}')
    return info


def font_smoke(api):
    """Exercise dynamically imported font tables in the actual frozen process."""
    api.replace({'version': 1, 'nodes': {'font': {'type': 'font', 'params': {
        'text': 'MiniHou O8', 'primitive_type': 'polygons', 'add_attributes': True}}}, 'display': 'font'})
    head, arrays, _ = api.preview('font', attr={'name': 'textsymbol', 'domain': 'prim'})
    require(head['points'] > 0 and head['tris'] > 0, f'font produced no geometry: {head}')
    require('attr' in arrays and not head.get('attr', {}).get('missing'), 'font textsymbol attribute missing')
    return {'points': head['points'], 'primitives': head['prims'], 'triangles': head['tris'],
            'textsymbol_attribute': head['attr']}


def native_smoke(api, internal):
    """Use Blender only as a disposable background worker with factory settings."""
    worker = internal / 'blender_addon/minihou_link/native_mesh_worker.py'
    catalog = internal / 'minihou/blender_mesh_catalog.py'
    require(worker.is_file(), f'packaged native worker missing: {worker}')
    require(catalog.is_file(), f'packaged native worker catalog missing: {catalog}')
    blenders = api.json('/api/blender').get('blenders', [])
    require(blenders, 'native packaged smoke requires an installed Blender')
    api.replace({'version': 1, 'nodes': {
        'box': {'type': 'box', 'params': {'size': [2, 2, 2]}},
        'bevel': {'type': 'native_bevel', 'inputs': {'geo': 'box'},
                  'params': {'amount': 0.1, 'segments': 2, 'result_group': 'native_smoke'}}}, 'display': 'bevel'})
    inspected = api.json('/api/inspect', {'node': 'bevel', 'detail': True})
    require(inspected.get('points', 0) > 8 and inspected.get('prims', 0) > 6,
            f'native bevel did not change box geometry: {inspected}')
    version = next((item.get('value') for item in inspected.get('attributes', [])
                    if item.get('domain') == 'detail' and item.get('name') == 'native_blender_version'), None)
    require(version, 'native worker did not report a Blender version')
    head, _, _ = api.preview('bevel', mask={'name': 'native_smoke', 'domain': 'point'})
    require(head.get('mask', {}).get('members', 0) > 0, 'native worker result-selection group missing')
    return {'blender_version': version, 'points': inspected['points'], 'primitives': inspected['prims'],
            'selected_points': head['mask']['members'], 'worker': str(worker), 'catalog': str(catalog)}


def viewport_smoke(api, browser, url, scratch):
    page = browser.new_page(viewport={'width': 1440, 'height': 1000})
    errors = []
    page.on('pageerror', lambda error: errors.append(str(error)))
    try:
        page.goto(url)
        page.wait_for_function('window.minihou?.S?.graph?.nodes?.expr')
        page.locator('.node[data-id="expr"] .title').click()
        page.wait_for_function('document.querySelectorAll(".rule-row").length===2')
        page.locator('.add-rule').click()
        page.wait_for_function('minihou.S.graph.nodes.expr.params.rules.length===3')
        row = page.locator('.rule-row').nth(2)
        row.locator('input[data-field="name"]').fill('browser_row')
        row.locator('input[data-field="name"]').press('Tab')
        page.wait_for_function('minihou.S.graph.nodes.expr.params.rules[2].name==="browser_row"')
        row.locator('input[data-field="expression"]').fill('@P.x == 0')
        row.locator('input[data-field="expression"]').press('Tab')
        page.wait_for_function('minihou.S.graph.nodes.expr.params.rules[2].expression==="@P.x == 0"')
        head, _, _ = api.preview('expr', mask={'name': 'browser_row', 'domain': 'point'})
        require(head['mask']['members'] == 9, 'browser rule edit did not cook the expected membership')
        page.locator('.rule-row').nth(2).get_by_label('Move rule up', exact=True).click()
        page.wait_for_function('minihou.S.graph.nodes.expr.params.rules[1].name==="browser_row"')
        page.locator('.rule-row').nth(1).get_by_role('button', name='Remove rule', exact=True).click()
        page.wait_for_function('minihou.S.graph.nodes.expr.params.rules.length===2')
        rule_result = {'added_edited_reordered_removed': True, 'cooked_members': 9}
        page.screenshot(path=str(scratch / 'group-rules.png'))

        baseline = acceleration(api)
        graph = {'version': 1, 'nodes': {
            'heavy_grid': {'type': 'grid', 'params': {'columns': 400, 'rows': 400}},
            'noise': {'type': 'noise', 'inputs': {'geo': 'heavy_grid'}, 'params': {'amplitude': [0, 0, 0.5], 'seed': 51}},
            'move': {'type': 'transform', 'inputs': {'geo': 'noise'}, 'params': {'translate': [0, 0, 0]}}}, 'display': 'move'}
        api.replace(graph)
        page.select_option('#pv-follow', 'display')
        page.wait_for_function('minihou.preview.primaryNode==="move" && minihou.preview.points===160000', timeout=120000)
        page.wait_for_function('minihou.preview.drawStats().draws>0')
        head, arrays, full_bytes = api.preview('move')
        require(head['tris'] > 100000 and 'chunks' in arrays, f'large preview chunk path not exercised: {head}')
        require('tri' in arrays and 'edge' in arrays, 'full preview omitted topology')
        page.evaluate('window.smokeTriangleArray=minihou.preview.geometry().tri; window.smokeEdgeArray=minihou.preview.geometry().edge; window.smokeBeforePositions=Array.from(minihou.preview.geometry().P.subarray(0,12));')
        start = time.perf_counter()
        api.json('/api/edit', {'ops': [{'op': 'set', 'id': 'move', 'params': {'translate': [1, 2, 3]}}], 'wait': True})
        page.wait_for_function('minihou.preview.lastUpload==="positions" && Math.abs(minihou.preview.geometry().P[0]-window.smokeBeforePositions[0]-1)<0.0001', timeout=120000)
        moved, updated, positions_bytes = api.preview('move', have=head['topology'])
        require(moved.get('same_topology') is True, 'server positions-only response not used')
        require(not {'tri', 'edge', 'tri_prim'}.intersection(updated), 'positions-only response resent topology')
        require(arrays['P'] != updated['P'] and len(arrays['P']) == len(updated['P']), 'positions were not updated')
        original_p = struct.unpack('<' + 'f' * (len(arrays['P']) // 4), arrays['P'])
        moved_p = struct.unpack('<' + 'f' * (len(updated['P']) // 4), updated['P'])
        maximum_error = max(abs(b - a - (1, 2, 3)[i % 3]) for i, (a, b) in enumerate(zip(original_p, moved_p)))
        require(maximum_error < 0.0001, f'wrong transform coordinates: maximum error {maximum_error}')
        viewport = page.evaluate('''()=>({last_upload:minihou.preview.lastUpload,
            triangles_reused:minihou.preview.geometry().tri===window.smokeTriangleArray,
            edges_reused:minihou.preview.geometry().edge===window.smokeEdgeArray,
            points:minihou.preview.points,tris:minihou.preview.tris,draw:minihou.preview.drawStats(),
            webgl:!!document.querySelector('#pv-canvas').getContext('webgl2')})''')
        require(viewport['triangles_reused'] and viewport['edges_reused'] and viewport['webgl'], f'viewport topology buffers not reused: {viewport}')
        after = acceleration(api)
        counts = after['successful_calls']
        delta = {name: count - baseline['successful_calls'].get(name, 0) for name, count in counts.items()}
        for kernel in ('noise_vec', 'fan_point_triangles', 'tri_morton', 'radix_order30', 'chunk_bounds'):
            require(delta.get(kernel, 0) > 0, f'packaged Numba kernel not executed by smoke: {kernel}: {delta}')
        require(not errors, f'browser errors: {errors}')
        page.screenshot(path=str(scratch / 'positions-only.png'))
        return {'rules': rule_result, 'viewport': viewport, 'full_payload_bytes': full_bytes,
                'positions_payload_bytes': positions_bytes, 'maximum_position_error': maximum_error,
                'update_seconds': round(time.perf_counter() - start, 3), 'kernel_call_delta': delta, 'backend': after}
    finally:
        page.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='JSON evidence file (success and failure)')
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--browser-executable', type=Path, help='optional explicit Chromium executable')
    args = parser.parse_args()
    exe, output = args.exe.resolve(), args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix='morning-packaged-smoke-', dir=output.parent))
    report = {'ok': False, 'exe': str(exe), 'scratch': str(scratch), 'started_at': time.strftime('%Y-%m-%dT%H:%M:%S%z')}
    process = None
    try:
        require(exe.is_file(), f'executable missing: {exe}')
        require((exe.parent / '_internal').is_dir(), 'portable _internal folder must sit beside MiniHou.exe')
        report['exe_sha256'] = hashlib.sha256(exe.read_bytes()).hexdigest()
        env = dict(os.environ)
        for name in ('PYTHONHOME', 'PYTHONPATH', 'MINIHOU_NO_NUMBA'):
            env.pop(name, None)
        env.update(MINIHOU_HOME=str(scratch / 'home'), LOCALAPPDATA=str(scratch / 'local'),
                   APPDATA=str(scratch / 'roaming'), NUMBA_CACHE_DIR=str(scratch / 'numba-cache'),
                   MINIHOU_PREFERENCES_FILE=str(scratch / 'preferences.json'), PYTHONUNBUFFERED='1')
        (scratch / 'preferences.json').write_text('{"length_unit":"m","unit_scale":1}', encoding='utf-8')
        with (scratch / 'stdout.log').open('w', encoding='utf-8') as log:
            process = subprocess.Popen([str(exe), 'app', '--no-browser', '--workspace', str(scratch / 'workspace'),
                                        '--http-port', '0', '--port', '0'], cwd=scratch, env=env, stdout=log, stderr=subprocess.STDOUT)
        report['pid'] = process.pid
        deadline, url = time.monotonic() + args.timeout, None
        while time.monotonic() < deadline:
            logs = ''.join(path.read_text(encoding='utf-8', errors='replace') for path in
                           (scratch / 'stdout.log', scratch / 'local/MiniHou/app.log') if path.exists())
            if process.poll() is not None:
                raise RuntimeError(f'executable exited early ({process.returncode}):\n{logs}')
            for line in logs.splitlines():
                if line.startswith('editor: '):
                    url = line.split('editor: ', 1)[1].strip()
            if url:
                break
            time.sleep(0.25)
        require(url, f'executable did not report an editor URL after {args.timeout}s')
        report['url'] = url
        api = Api(url, args.timeout)
        state = api.json('/api/state')
        engine = state.get('engine', {})
        require(engine.get('frozen') is True, f'test must target the frozen executable: {engine}')
        module_path = Path(engine.get('module_path', '')).resolve()
        require(module_path.is_relative_to(exe.parent / '_internal'), f'packaged module loaded outside _internal: {module_path}')
        report['engine'] = engine
        report['initial_backend'] = acceleration(api)
        required = {'array', 'circle_array', 'grid_array', 'array_between_two_positions',
                    'array_linear_points', 'array_circle_points', 'array_grid_points', 'array_between_points'}
        require(required <= {node['name'] for node in state['nodes']}, 'bundled array helpers missing')
        addon = api.raw('/addon/minihou_link.zip')
        require(addon.startswith(b'PK') and len(addon) > 10000, 'packaged Blender addon unavailable')
        report['addon_zip_bytes'] = len(addon)
        report['groups'] = group_smoke(api)
        from playwright.sync_api import sync_playwright
        with sync_playwright() as pw:
            options = {'headless': True}
            if args.browser_executable:
                options['executable_path'] = str(args.browser_executable.resolve())
            browser = pw.chromium.launch(**options)
            try:
                report['browser'] = viewport_smoke(api, browser, url, scratch)
            finally:
                browser.close()
        report['font'] = font_smoke(api)
        report['native'] = native_smoke(api, exe.parent / '_internal')
        report['ok'] = True
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
        report['traceback'] = traceback.format_exc()
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(10)
        report['finished_at'] = time.strftime('%Y-%m-%dT%H:%M:%S%z')
        output.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps({'ok': report['ok'], 'report': str(output), 'error': report.get('error')}))
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
