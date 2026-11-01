#!/usr/bin/env python3
"""Exercise an original CS3 through the actual worker; optional live source search."""
import argparse
import asyncio
import hashlib
import json
import os
from pathlib import Path


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('extension', type=Path)
    parser.add_argument('--query')
    parser.add_argument('--result', type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    resources = Path(os.environ.get('CLOUDSTREAM_RESOURCE_DIR', root / 'src-tauri/resources'))
    extension = args.extension.resolve()
    plugin_id = hashlib.sha256(extension.read_bytes()).hexdigest()
    process = await asyncio.create_subprocess_exec(
        str(resources / 'jre/bin/java'), '-Xmx512m', '-jar', str(resources / 'worker/worker.jar'),
        stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.DEVNULL,
    )
    sequence = 0

    async def request(method, params):
        nonlocal sequence
        sequence += 1
        process.stdin.write((json.dumps({'jsonrpc': '2.0', 'id': sequence, 'method': method, 'params': params}) + '\n').encode())
        await process.stdin.drain()
        reply = json.loads(await asyncio.wait_for(process.stdout.readline(), 30))
        assert reply['id'] == sequence
        if 'error' in reply:
            raise RuntimeError(reply['error']['message'])
        return reply['result']

    try:
        providers = await request('loadPlugin', {'pluginId': plugin_id, 'path': str(extension)})
        assert providers, 'The extension did not register a media source'
        result = {'providers': providers}
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
