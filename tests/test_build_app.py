import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from tools import build_app

class BuildDeliveryTests(unittest.TestCase):
    def test_native_worker_and_catalog_are_delivered_as_source_data(self):
        import os
        with tempfile.TemporaryDirectory() as directory, patch.object(build_app.subprocess, 'run') as run:
            build_app.build(False, Path(directory))
        command=run.call_args.args[0]
        data=[command[i+1] for i, part in enumerate(command[:-1]) if part=='--add-data']
        self.assertIn(str(build_app.ROOT/'blender_addon'/'minihou_link')+os.pathsep+'blender_addon/minihou_link',data)
        self.assertTrue((build_app.ROOT/'blender_addon'/'minihou_link'/'native_mesh_worker.py').is_file())
        self.assertIn(str(build_app.ROOT/'minihou'/'blender_mesh_catalog.py')+os.pathsep+'minihou',data)

    def test_default_is_unpacked_and_preserves_previous(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'build').mkdir();dist=root/'dist';(dist/'MiniHou').mkdir(parents=True)
            (dist/'MiniHou'/'old.txt').write_text('old',encoding='utf-8')
            def fake_build(accel, stage):
                folder=stage/'MiniHou';folder.mkdir();(folder/'MiniHou.exe').write_text('new',encoding='utf-8');return folder
            with patch.object(build_app,'ROOT',root),patch.object(build_app,'build',side_effect=fake_build),patch.object(build_app,'smoke') as smoke,patch.object(build_app.shutil,'make_archive') as archive,patch('sys.argv',['build_app','--output',str(dist)]):
                self.assertEqual(build_app.main(),0)
                smoke.assert_called_once();archive.assert_not_called()
            self.assertEqual([p.name for p in dist.iterdir()],['MiniHou'])
            self.assertTrue((dist/'MiniHou'/'MiniHou.exe').exists())
            self.assertEqual(len(list((root/'build').glob('app-previous-*/MiniHou/old.txt'))),1)

    def test_locked_application_is_left_intact(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);(root/'build').mkdir();dist=root/'dist';(dist/'MiniHou').mkdir(parents=True)
            original=dist/'MiniHou'/'old.txt';original.write_text('old',encoding='utf-8')
            def fake_build(accel,stage):
                folder=stage/'MiniHou';folder.mkdir();return folder
            with patch.object(build_app,'ROOT',root),patch.object(build_app,'build',side_effect=fake_build),patch.object(build_app,'smoke'),patch.object(Path,'rename',side_effect=PermissionError('locked')),patch('sys.argv',['build_app','--output',str(dist)]):
                with self.assertRaises(PermissionError):build_app.main()
            self.assertEqual(original.read_text(encoding='utf-8'),'old')
