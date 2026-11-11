#!/usr/bin/env python3
"""The concurrency gate uses the same single-request batch test as live metrics."""
from pathlib import Path
import runpy

runpy.run_path(str(Path(__file__).with_name('test-live-metrics.py')), run_name='__main__')
