// The MCP server editor's pure logic: a saved server as editor text
// (McpServerDraft.of), why a draft can't be saved (validateMcpServer) and the
// draft back as BubblePaw JSON on top of the saved server (mcpServerJson).
// Table tests: duplicate names, command and URL servers, a change of
// transport, every validation message, and keys the editor doesn't know,
// which must survive load -> edit -> save unchanged.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

McpServerDraft _cmd({
  String name = 'files',
  String command = 'npx',
  String args = '',
  String env = '',
  String url = '',
  String headers = '',
}) => McpServerDraft(
  name: name,
  command: command,
  args: args,
  env: env,
  url: url,
  headers: headers,
);

McpServerDraft _url({
  String name = 'docs',
  String url = 'https://example.com/mcp',
  String headers = '',
  String command = '',
  String env = '',
}) => McpServerDraft(
  name: name,
  isUrl: true,
  url: url,
  headers: headers,
  command: command,
  env: env,
);

void main() {
  group('validateMcpServer', () {
    final cases = <(String, McpServerDraft, List<String>, String?)>[
      ('a command server', _cmd(), const [], null),
      ('a URL server', _url(), const [], null),
      ('no name', _cmd(name: ''), const [], 'Give the server a name.'),
      ('a blank name', _cmd(name: '   '), const [], 'Give the server a name.'),
      (
        'a taken name',
        _cmd(name: 'docs'),
        const ['docs'],
        'A server named docs already exists.',
      ),
      (
        'a taken name with spaces around it',
        _cmd(name: '  docs '),
        const ['docs'],
        'A server named docs already exists.',
      ),
      (
        'a taken name on a URL server',
        _url(name: 'files'),
        const ['files', 'other'],
        'A server named files already exists.',
      ),
      ('a name that differs in case', _cmd(name: 'Docs'), const ['docs'], null),
      ('its own name (not among the others)', _cmd(), const ['docs'], null),
      (
        'a command server without a command',
        _cmd(command: '  '),
        const [],
        'Enter the command that starts it.',
      ),
      (
        'an environment line without =',
        _cmd(env: 'ROOT'),
        const [],
        'Write each environment variable as KEY=value, one per line.',
      ),
      (
        'an environment line without a key',
        _cmd(env: '=value'),
        const [],
        'Write each environment variable as KEY=value, one per line.',
      ),
      (
        'blank environment lines and an empty value',
        _cmd(env: '\n  \nA=\nB = 1 \n'),
        const [],
        null,
      ),
      (
        'a command server ignores the URL fields',
        _cmd(url: '', headers: 'junk'),
        const [],
        null,
      ),
      (
        'a URL server without a URL',
        _url(url: ' '),
        const [],
        'Enter the server URL.',
      ),
      (
        'a header line without =',
        _url(headers: 'Authorization'),
        const [],
        'Write each header as Header=ENV_VAR, one per line.',
      ),
      (
        'a header line without a name',
        _url(headers: '=GH_TOKEN'),
        const [],
        'Write each header as Header=ENV_VAR, one per line.',
      ),
      (
        'a header without a variable',
        _url(headers: 'Authorization= '),
        const [],
        'Header Authorization needs an environment variable.',
      ),
      (
        'a header variable with another ending',
        _url(headers: 'Authorization=GH_SECRET'),
        const [],
        'GH_SECRET must end in _API_KEY, _KEY or _TOKEN.',
      ),
      (
        'header variables ending in _API_KEY, _KEY and _TOKEN',
        _url(headers: 'A=X_API_KEY\nB=Y_KEY\n\nC = Z_TOKEN'),
        const [],
        null,
      ),
      (
        'a URL server ignores the command fields',
        _url(command: '', env: 'junk'),
        const [],
        null,
      ),
    ];
    for (final (what, draft, others, error) in cases) {
      test(what, () => expect(validateMcpServer(draft, others), error));
    }
  });

  group('mcpServerJson', () {
    final cases =
        <(String, McpServerDraft, Map<String, dynamic>, Map<String, dynamic>)>[
          (
            'a new command server',
            _cmd(args: '-y\n@example/files', env: 'ROOT=/data'),
            const {},
            {
              'enabled': true,
              'command': 'npx',
              'args': ['-y', '@example/files'],
              'env': {'ROOT': '/data'},
            },
          ),
          (
            'text is trimmed and blank lines dropped',
            _cmd(
              command: ' npx ',
              args: '\n  -y \n\n x\n',
              env: ' A = b=c \n\n',
            ),
            const {},
            {
              'enabled': true,
              'command': 'npx',
              'args': ['-y', 'x'],
              'env': {'A': 'b=c'},
            },
          ),
          (
            'a new URL server',
            _url(headers: 'Authorization=GH_TOKEN'),
            const {},
            {
              'enabled': true,
              'url': 'https://example.com/mcp',
              'headers_from': {'Authorization': 'GH_TOKEN'},
            },
          ),
          (
            'a turned-off server stays off',
            _cmd(),
            const {'enabled': false, 'command': 'old'},
            {'enabled': false, 'command': 'npx'},
          ),
          (
            'cleared arguments and environment are removed',
            _cmd(),
            const {
              'enabled': true,
              'command': 'npx',
              'args': ['-y'],
              'env': {'A': '1'},
            },
            {'enabled': true, 'command': 'npx'},
          ),
          (
            'cleared headers are removed',
            _url(),
            const {
              'enabled': true,
              'url': 'https://example.com/mcp',
              'headers_from': {'A': 'X_KEY'},
            },
            {'enabled': true, 'url': 'https://example.com/mcp'},
          ),
          (
            'unknown keys stay on a command server',
            _cmd(command: 'uvx'),
            const {
              'command': 'npx',
              'enabled': true,
              'cwd': '/work',
              'env_from': {'TOKEN': 'GH_TOKEN'},
              'call_timeout_secs': 60,
              'future': {
                'nested': [1, 2],
              },
            },
            {
              'command': 'uvx',
              'enabled': true,
              'cwd': '/work',
              'env_from': {'TOKEN': 'GH_TOKEN'},
              'call_timeout_secs': 60,
              'future': {
                'nested': [1, 2],
              },
            },
          ),
          (
            'unknown keys stay on a URL server',
            _url(),
            const {
              'url': 'https://old.example.com/mcp',
              'enabled': true,
              'startup_timeout_secs': 30,
            },
            {
              'url': 'https://example.com/mcp',
              'enabled': true,
              'startup_timeout_secs': 30,
            },
          ),
          (
            'command to URL drops the command keys and keeps the rest',
            _url(headers: 'Authorization=GH_TOKEN'),
            const {
              'command': 'npx',
              'args': ['-y'],
              'env': {'A': '1'},
              'env_from': {'B': 'B_TOKEN'},
              'cwd': '/work',
              'enabled': false,
              'call_timeout_secs': 60,
            },
            {
              'enabled': false,
              'call_timeout_secs': 60,
              'url': 'https://example.com/mcp',
              'headers_from': {'Authorization': 'GH_TOKEN'},
            },
          ),
          (
            'URL to command drops the URL keys and keeps the rest',
            _cmd(args: 'serve'),
            const {
              'url': 'https://example.com/mcp',
              'headers_from': {'A': 'X_KEY'},
              'enabled': true,
              'startup_timeout_secs': 30,
            },
            {
              'enabled': true,
              'startup_timeout_secs': 30,
              'command': 'npx',
              'args': ['serve'],
            },
          ),
        ];
    for (final (what, draft, base, want) in cases) {
      test(what, () => expect(mcpServerJson(draft, base), want));
    }

    test('the saved server is not changed in place', () {
      final base = <String, dynamic>{'command': 'npx', 'enabled': true};
      mcpServerJson(_url(), base);
      expect(base, {'command': 'npx', 'enabled': true});
    });
  });

  group('McpServerDraft.of', () {
    test('a command server as text', () {
      final d = McpServerDraft.of('files', {
        'command': 'npx',
        'args': ['-y', 2],
        'env': {'A': '1', 'B': 'x=y'},
        'enabled': false,
      });
      expect(
        [d.name, d.isUrl, d.command, d.args, d.env, d.url, d.headers],
        ['files', false, 'npx', '-y\n2', 'A=1\nB=x=y', '', ''],
      );
    });

    test('a URL server as text', () {
      final d = McpServerDraft.of('docs', {
        'url': 'https://example.com/mcp',
        'headers_from': {'Authorization': 'GH_TOKEN'},
      });
      expect(
        [d.name, d.isUrl, d.command, d.args, d.env, d.url, d.headers],
        [
          'docs',
          true,
          '',
          '',
          '',
          'https://example.com/mcp',
          'Authorization=GH_TOKEN',
        ],
      );
    });

    test('values of the wrong type read as empty', () {
      final d = McpServerDraft.of('odd', {
        'command': 3,
        'args': 'not a list',
        'env': ['A=1'],
        'url': false,
      });
      expect(
        [d.isUrl, d.command, d.args, d.env, d.url],
        [false, '', '', '', ''],
      );
    });
  });

  // Saved servers opened in the editor and saved untouched come back byte
  // for byte, keys the editor doesn't know included.
  group('load -> edit -> save round trip', () {
    final saved = <String, Map<String, dynamic>>{
      'files': {
        'command': 'npx',
        'args': ['-y', '@example/files'],
        'env': {'ROOT': '/data'},
        'enabled': true,
        'call_timeout_secs': 60,
      },
      'scripted': {
        'enabled': false,
        'command': 'uvx',
        'cwd': '/work',
        'env_from': {'TOKEN': 'GH_TOKEN'},
        'future_key': {
          'nested': [1, 'two', null],
        },
      },
      'docs': {
        'url': 'https://hub.example.com/mcp',
        'headers_from': {'Authorization': 'HUB_TOKEN'},
        'enabled': true,
        'startup_timeout_secs': 30,
      },
    };
    for (final MapEntry(key: name, value: json) in saved.entries) {
      test(name, () {
        final draft = McpServerDraft.of(name, json);
        expect(validateMcpServer(draft, const []), isNull);
        expect(jsonEncode(mcpServerJson(draft, json)), jsonEncode(json));
      });
    }

    test('editing one field changes only that field', () {
      final json = saved['scripted']!;
      final draft = McpServerDraft.of('scripted', json)..args = 'run\nserver';
      final out = mcpServerJson(draft, json);
      expect(out.keys.toList(), [...json.keys, 'args']);
      expect(out['args'], ['run', 'server']);
      expect(jsonEncode({...out}..remove('args')), jsonEncode(json));
    });
  });
}
