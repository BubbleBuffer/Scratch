// The MCP servers section's logic without its layout (McpServersController)
// and the runner pickers' options (agent_runner_choices.dart): the editor
// opens with a saved server's text, a refused save writes nothing, saves keep
// the list order and the keys the editor doesn't know, turning a server off
// or removing it leaves the others as saved, and each state reads as before.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/agent_runner_choices.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/ai/ai_settings.dart' show AgentHost;

const McpServers _saved = {
  'files': {
    'command': 'npx',
    'args': ['-y', '@example/files'],
    'enabled': true,
    'call_timeout_secs': 60,
  },
  'docs': {
    'url': 'https://hub.example.com/mcp',
    'headers_from': {'Authorization': 'HUB_TOKEN'},
    'enabled': false,
    'startup_timeout_secs': 30,
  },
  'last': {'command': 'uvx', 'enabled': true},
};

/// A controller over a copy of [_saved]; [writes] collects what it saves.
(McpServersController, List<McpServers>) _controller({
  List<AgentMcpServer>? Function()? status,
}) {
  var servers = McpServers.of(_saved);
  final writes = <McpServers>[];
  final c = McpServersController(
    read: () => servers,
    write: (next) {
      writes.add(next);
      servers = next;
    },
    status: () async => status?.call(),
  );
  addTearDown(c.dispose);
  return (c, writes);
}

void main() {
  group('McpServersController', () {
    test('opens the editor with a saved server, or empty for a new one', () {
      final (c, writes) = _controller();
      c.edit('files');
      expect(
        [c.editing, c.original, c.isUrl, c.name.text, c.command.text],
        [true, 'files', false, 'files', 'npx'],
      );
      expect(c.args.text, '-y\n@example/files');
      c.edit('docs');
      expect(
        [c.isUrl, c.url.text, c.headers.text],
        [true, 'https://hub.example.com/mcp', 'Authorization=HUB_TOKEN'],
      );
      c.edit(null);
      expect(
        [c.original, c.isUrl, c.name.text, c.url.text],
        [null, false, '', ''],
      );
      expect(writes, isEmpty);
    });

    test('a refused save shows why and writes nothing', () {
      final (c, writes) = _controller();
      c.edit(null);
      c.save();
      expect(c.editorError, 'Give the server a name.');
      c.name.text = ' docs ';
      c.command.text = 'npx';
      c.save();
      expect(c.editorError, 'A server named docs already exists.');
      expect(c.editing, isTrue);
      expect(writes, isEmpty);
    });

    test('a new server goes at the end', () {
      final (c, writes) = _controller();
      c.edit(null);
      c.name.text = 'new';
      c.command.text = 'run';
      c.save();
      expect(writes.single.keys, ['files', 'docs', 'last', 'new']);
      expect(writes.single['new'], {'enabled': true, 'command': 'run'});
      expect([c.editing, c.original, c.editorError], [false, null, null]);
    });

    test('saving untouched writes the list back byte for byte', () {
      final (c, writes) = _controller();
      for (final name in _saved.keys) {
        c.edit(name);
        c.save();
      }
      expect(writes, hasLength(3));
      for (final w in writes) {
        expect(jsonEncode(w), jsonEncode(_saved));
      }
    });

    test('a rename keeps the place, the other keys and its own name free', () {
      final (c, writes) = _controller();
      c.edit('docs');
      c.name.text = 'hub docs';
      c.save();
      expect(writes.single.keys, ['files', 'hub docs', 'last']);
      expect(jsonEncode(writes.single['hub docs']), jsonEncode(_saved['docs']));
    });

    test('a change of type keeps the keys the editor does not show', () {
      final (c, writes) = _controller();
      c.edit('files');
      c.isUrl = true;
      c.url.text = 'https://example.com/mcp';
      c.save();
      expect(writes.single['files'], {
        'enabled': true,
        'call_timeout_secs': 60,
        'url': 'https://example.com/mcp',
      });
    });

    test('turning a server on or off changes only its enabled key', () {
      final (c, writes) = _controller();
      c.setEnabled('docs', true);
      expect(writes.single.keys, ['files', 'docs', 'last']);
      expect(writes.single['docs'], {..._saved['docs']!, 'enabled': true});
      expect(writes.single['files'], same(_saved['files']));
    });

    test('removing a server closes its editor, not another one', () {
      final (c, writes) = _controller();
      c.edit('files');
      c.remove('last');
      expect([c.editing, c.original], [true, 'files']);
      c.remove('files');
      expect([c.editing, c.original], [false, null]);
      expect(writes.last.keys, ['docs']);
      expect(jsonEncode(writes.last['docs']), jsonEncode(_saved['docs']));
    });

    test('states read as before', () async {
      final (c, _) = _controller(
        status: () => const [
          AgentMcpServer(name: 'a', state: 'running', tools: 1),
          AgentMcpServer(name: 'b', state: 'running', tools: 3),
          AgentMcpServer(name: 'c', state: 'failed', tools: 0),
          AgentMcpServer(name: 'd', state: 'failed', tools: 0, error: ' boom '),
          AgentMcpServer(name: 'e', state: 'off', tools: 0),
        ],
      );
      expect(c.stateOf('a', true), isNull, reason: 'not read yet');
      await c.refresh();
      expect(
        [
          for (final (n, on) in [
            ('a', true),
            ('b', true),
            ('c', true),
            ('d', true),
            ('e', true),
            ('missing', true),
            ('missing', false),
          ])
            c.stateOf(n, on),
        ],
        [
          ('Running, 1 tool', false),
          ('Running, 3 tools', false),
          ('Failed to start', true),
          ('Failed: boom', true),
          ('Off', false),
          ('Not started', false),
          ('Off', false),
        ],
      );
    });

    test('a refresh notifies only when the states changed', () async {
      var list = const [AgentMcpServer(name: 'a', state: 'off', tools: 0)];
      final (c, _) = _controller(status: () => list);
      var notified = 0;
      c.addListener(() => notified++);
      await c.refresh();
      await c.refresh();
      expect(notified, 1);
      list = const [AgentMcpServer(name: 'a', state: 'running', tools: 2)];
      await c.refresh();
      expect([notified, c.serverStates], [2, list]);
    });
  });

  group('runner choices', () {
    test('hub model options keep a choice the hub no longer lists', () {
      expect(hubModelOptions(['a', 'b'], null), [null, 'a', 'b']);
      expect(hubModelOptions(['a', 'b'], 'b'), [null, 'a', 'b']);
      expect(hubModelOptions(['a'], 'gone'), [null, 'a', 'gone']);
      expect(hubModelOptions([], 'gone'), [null, 'gone']);
    });

    test('hub model labels', () {
      String label(String id) => 'Model $id';
      expect(hubModelLabel(null, null, label), 'Hub default');
      expect(hubModelLabel(null, 'x', label), 'Hub default (Model x)');
      expect(hubModelLabel('y', 'x', label), 'Model y');
    });

    test('the computer model falls back to the first one listed', () {
      expect(computerModelValue(['a', 'b'], 'b'), 'b');
      expect(computerModelValue(['a', 'b'], 'gone'), 'a');
      expect(computerModelValue(['a', 'b'], null), 'a');
    });

    test('runner help', () {
      expect(
        runnerHelp(AgentHost.hub),
        'Works the same from every device, including your phone. Uses the models signed in on the hub.',
      );
      expect(
        runnerHelp(AgentHost.local),
        'Uses this computer’s models, and can work in its folders and programs.',
      );
    });
  });
}
