#!/usr/bin/env python3
"""Exercise an original CS3 through the actual worker; optional live source search or inspection."""
import argparse
import asyncio
import hashlib
import json
import os
import sys
from pathlib import Path


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('extension', type=Path)
    parser.add_argument('--query')
    parser.add_argument('--inspect', action='store_true', help='Inspect package capabilities without loading code')
    parser.add_argument('--result', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    resources = Path(os.environ.get('CLOUDSTREAM_RESOURCE_DIR', root / 'src-tauri/resources'))
    java_cmd = str(resources / 'jre/bin/java') if (resources / 'jre/bin/java').is_file() else 'java'
    extension = args.extension.resolve()
    plugin_id = hashlib.sha256(extension.read_bytes()).hexdigest()
    process = await asyncio.create_subprocess_exec(
        java_cmd, '-Xmx512m', '-jar', str(resources / 'worker/worker.jar'),
        stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL,
    )
    sequence = 0

    async def request(method, params):
        nonlocal sequence
        sequence += 1
        process.stdin.write((json.dumps({'jsonrpc': '2.0', 'id': sequence, 'method': method, 'params': params}) + '\n').encode())
        await process.stdin.drain()
        while True:
            line = await asyncio.wait_for(process.stdout.readline(), 30)
            if not line:
                raise RuntimeError("Worker disconnected")
            reply = json.loads(line.decode().strip())
            if reply.get('id') == sequence:
                if 'error' in reply:
                    raise RuntimeError(reply['error']['message'])
                return reply['result']

    try:
        if args.inspect:
            report = await request('inspectPackage', {'path': str(extension)})
            print(f"PASS: package inspection complete: {report.get('format')} format, type: {report.get('pluginType')}")
            if args.result:
                args.result.write_text(json.dumps(report, ensure_ascii=False, indent=2))
            else:
                print(json.dumps(report, indent=2))
            return

        providers = await request('loadPlugin', {'pluginId': plugin_id, 'path': str(extension)})
        states = await request('getPluginStates', {})
        assert providers, 'The extension did not register a media source'
        result = {'providers': providers, 'states': states}
        if args.query:
            result['search'] = await request('search', {'providerId': providers[0]['id'], 'query': args.query, 'page': 1})
            print(f"PASS: original CS3 loaded; {len(result['search']['items'])} live search results")
        else:
            print(f"PASS: original CS3 loaded; {len(providers)} media sources registered")
        if args.result:
            args.result.write_text(json.dumps(result, ensure_ascii=False, indent=2))
    finally:
        # OkHttp keeps idle dispatcher threads alive after stdin EOF; stop the
        # owned worker explicitly once all protocol replies have been read.
        if process.returncode is None:
            process.terminate()
        await process.wait()


if __name__ == '__main__':
    asyncio.run(main())
