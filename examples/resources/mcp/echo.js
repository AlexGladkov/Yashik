// A small dependency-free stdio MCP server for the installation smoke example.
// It implements initialize, ping, tools/list and one echo tool.
const readline = require('node:readline');
const input = readline.createInterface({ input: process.stdin });
input.on('line', (line) => {
  let request;
  try { request = JSON.parse(line); } catch { return; }
  if (request.id === undefined) return;
  const response = { jsonrpc: '2.0', id: request.id };
  switch (request.method) {
    case 'initialize':
      response.result = {
        protocolVersion: request.params?.protocolVersion || '2024-11-05',
        capabilities: { tools: {} },
        serverInfo: { name: 'yashik-example-echo', version: '1.0.0' },
      };
      break;
    case 'ping': response.result = {}; break;
    case 'tools/list':
      response.result = { tools: [{
        name: 'echo', description: 'Return the provided text.',
        inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
      }] };
      break;
    case 'tools/call':
      if (request.params?.name !== 'echo') {
        response.error = { code: -32602, message: 'Unknown tool' };
      } else {
        response.result = { content: [{ type: 'text', text: String(request.params.arguments?.text || '') }] };
      }
      break;
    default: response.error = { code: -32601, message: 'Method not found' };
  }
  process.stdout.write(JSON.stringify(response) + '\n');
});
