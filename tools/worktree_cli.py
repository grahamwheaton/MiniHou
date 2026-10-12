"""Run this checkout's MiniHou CLI even with an embedded Python _pth file."""
import pathlib
import runpy
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
runpy.run_module('minihou',run_name='__main__')
