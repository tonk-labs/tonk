"""Pinned mcp==1.26.0; fixture credentials arrive over stdin, never argv/logs."""
import asyncio
import json
import sys
from mcp import ClientSession
from mcp.client.streamable_http import streamablehttp_client

async def main():
    config = json.load(sys.stdin)
    async with streamablehttp_client(config['url'], headers={'Authorization': 'Bearer ' + config['token']}) as (read, write, _):
        async with ClientSession(read, write) as session:
            initialized = await session.initialize()
            assert initialized.protocolVersion == '2025-11-25'
            assert sorted(t.name for t in (await session.list_tools()).tools) == ['tonk_evaluate', 'tonk_guide', 'tonk_list_spaces', 'tonk_query']
            spaces = await session.call_tool('tonk_list_spaces', {})
            assert spaces.structuredContent['spaces'][0]['subject'] == config['space']
            resources = await session.list_resources()
            assert resources.resources
            await session.read_resource(resources.resources[0].uri)
            written = await session.call_tool('tonk_evaluate', {'space': config['space'], 'document': 'integration-task!:\n  this: urn:test:python\n  title: Python SDK\n  done: false\n'})
            assert not written.isError
            result = await session.call_tool('tonk_query', {'space': config['space'], 'document': 'integration-task:\n  this: urn:test:python\n  title: ?title\n'})
            assert not result.isError
            assert 'Python SDK' in json.dumps(result.structuredContent)
    print('Python MCP 1.26.0 protocol 2025-11-25: list, guide, write, readback passed')

asyncio.run(main())
