#!/usr/bin/env python3
"""Measure warm HTTP latency, throughput, server RSS and idle-connection threads."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import http.client
import json
import platform
import resource
import socket
import sqlite3
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path


def threads(pid):
    if sys.platform == 'darwin':
        return len(subprocess.check_output(['ps', '-M', '-p', str(pid)], text=True).splitlines()) - 1
    if sys.platform == 'linux':
        return int(next(line.split()[1] for line in Path(f'/proc/{pid}/status').read_text().splitlines() if line.startswith('Threads:')))
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('database', type=Path)
    parser.add_argument('--bagkit', type=Path, default=Path('target/release/bagkit'))
    parser.add_argument('--path', help='defaults to coordinates near the middle dwelling')
    parser.add_argument('--requests', type=int, default=200)
    parser.add_argument('--concurrency', type=int, default=8)
    parser.add_argument('--workers', type=int, default=4)
    parser.add_argument('--idle-connections', type=int, default=0)
    args = parser.parse_args()
    if args.requests < args.concurrency or args.concurrency < 1 or args.workers < 1 or args.idle_connections < 0:
        parser.error('requests must be >= positive concurrency; workers must be positive; idle-connections must be nonnegative')
    database = args.database.resolve()
    with sqlite3.connect(f'{database.as_uri()}?mode=ro', uri=True) as db:
        lat, lon = db.execute("""
            SELECT breedtegraad, lengtegraad
            FROM adresseerbare_objecten
            WHERE breedtegraad IS NOT NULL
            ORDER BY id
            LIMIT 1
            OFFSET (
                SELECT MIN(COUNT(*) - 1, COUNT(*) / 2 + 49)
                FROM adresseerbare_objecten
                WHERE breedtegraad IS NOT NULL
            )
        """).fetchone()
    path = args.path or f'/v1/adressen/_dichtstbij?punt={lon+0.000001},{lat+0.000001}'
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    server = subprocess.Popen([str(args.bagkit.resolve()), 'serve', str(port), '--host', '127.0.0.1', '--database', str(database), '--workers', str(args.workers)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    idle = []
    try:
        for _ in range(500):
            if server.poll() is not None:
                raise RuntimeError('HTTP server exited before startup')
            try:
                with socket.create_connection(('127.0.0.1', port), timeout=.1):
                    break
            except OSError:
                time.sleep(.01)
        else:
            raise RuntimeError('HTTP server did not start')
        time.sleep(.05)
        baseline_threads = threads(server.pid)
        for _ in range(args.idle_connections):
            idle.append(socket.create_connection(('127.0.0.1', port), timeout=1))
        time.sleep(.05)
        idle_threads = threads(server.pid)
        barrier = threading.Barrier(args.concurrency + 1)
        def worker(count):
            connection = http.client.HTTPConnection('127.0.0.1', port, timeout=30)
            latencies = []
            def request():
                connection.request('GET', path)
                response = connection.getresponse()
                body = response.read()
                assert response.status == 200, (response.status, body)
                assert isinstance(json.loads(body), list)
            try:
                for _ in range(5):
                    request()
                barrier.wait(timeout=30)
                for _ in range(count):
                    started = time.perf_counter()
                    request()
                    latencies.append(time.perf_counter()-started)
            except Exception:
                barrier.abort()
                raise
            finally:
                connection.close()
            return latencies
        with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
            futures = [pool.submit(worker, args.requests//args.concurrency + (i < args.requests%args.concurrency)) for i in range(args.concurrency)]
            barrier.wait(timeout=30)
            started = time.perf_counter()
            latencies = sorted(value for future in futures for value in future.result())
            elapsed = time.perf_counter()-started
    finally:
        for client in idle:
            client.close()
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
    unit = 1024**2 if sys.platform == 'darwin' else 1024
    print(json.dumps({'os': platform.platform(), 'machine': platform.machine(), 'database_mib': database.stat().st_size/1024**2, 'path': path, 'requests': args.requests, 'concurrency': args.concurrency, 'workers': args.workers, 'elapsed_seconds': elapsed, 'requests_per_second': args.requests/elapsed, 'median_ms': statistics.median(latencies)*1000, 'p95_ms': latencies[min(len(latencies)-1, int(len(latencies)*.95))]*1000, 'server_peak_rss_mib': resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss/unit, 'threads_before_idle': baseline_threads, 'idle_connections': args.idle_connections, 'threads_after_idle': idle_threads, 'note': 'warm keep-alive requests include client JSON decoding; peak RSS is the server child; no cold-cache claim'}))


if __name__ == '__main__':
    main()
