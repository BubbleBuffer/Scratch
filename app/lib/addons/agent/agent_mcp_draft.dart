// The MCP server editor's data, without widgets: one server as editor text
// ([McpServerDraft]), why a draft can't be saved ([validateMcpServer]), the
// draft back as BubblePaw JSON on top of the saved server ([mcpServerJson]),
// and the edits to the device's server list. The JSON format is BubblePaw's
// `McpServerSettings`; keys the editor doesn't show are kept as saved.

/// One server as the editor shows it.
class McpServerDraft {
  McpServerDraft({
    this.name = '',
    this.isUrl = false,
    this.command = '',
    this.args = '',
    this.env = '',
    this.url = '',
    this.headers = '',
  });

  /// [json] (BubblePaw's `McpServerSettings`) named [name], as text.
  factory McpServerDraft.of(String name, Map<String, dynamic> json) {
    String lines(Object? map) => map is Map
        ? [for (final e in map.entries) '${e.key}=${e.value}'].join('\n')
        : '';
    final args = json['args'];
    return McpServerDraft(
      name: name,
      isUrl: json['url'] is String,
      command: json['command'] is String ? json['command'] as String : '',
      args: args is List ? args.map((a) => '$a').join('\n') : '',
      env: lines(json['env']),
      url: json['url'] is String ? json['url'] as String : '',
      headers: lines(json['headers_from']),
    );
  }

  String name, command, args, env, url, headers;
  bool isUrl;

  /// What the server runs, in one line: its URL, or its command with its
  /// arguments.
  String get summary => isUrl
      ? url
      : [command, ...mcpLines(args)].where((p) => p.isNotEmpty).join(' ');
}

/// Non-empty trimmed lines of [text].
List<String> mcpLines(String text) => [
  for (final l in text.split('\n'))
    if (l.trim().isNotEmpty) l.trim(),
];

/// `KEY=value` lines as a map; null when a line has no `=` or no key.
Map<String, String>? _pairs(String text) {
  final out = <String, String>{};
  for (final l in mcpLines(text)) {
    final i = l.indexOf('=');
    if (i <= 0) return null;
    out[l.substring(0, i).trim()] = l.substring(i + 1).trim();
  }
  return out;
}

/// Why [d] can't be saved among [names] (the other servers' names), or null.
String? validateMcpServer(McpServerDraft d, Iterable<String> names) {
  final name = d.name.trim();
  if (name.isEmpty) return 'Give the server a name.';
  if (names.contains(name)) return 'A server named $name already exists.';
  if (d.isUrl) {
    if (d.url.trim().isEmpty) return 'Enter the server URL.';
    final headers = _pairs(d.headers);
    if (headers == null) {
      return 'Write each header as Header=ENV_VAR, one per line.';
    }
    for (final e in headers.entries) {
      final v = e.value;
      if (v.isEmpty) return 'Header ${e.key} needs an environment variable.';
      if (!v.endsWith('_API_KEY') &&
          !v.endsWith('_KEY') &&
          !v.endsWith('_TOKEN')) {
        return '$v must end in _API_KEY, _KEY or _TOKEN.';
      }
    }
  } else {
    if (d.command.trim().isEmpty) return 'Enter the command that starts it.';
    if (_pairs(d.env) == null) {
      return 'Write each environment variable as KEY=value, one per line.';
    }
  }
  return null;
}

/// [d] as BubblePaw JSON, on top of [base] (the server as saved), so keys
/// the editor doesn't show stay as they are. Keys of the other kind go:
/// a URL server can't have a command, arguments, environment or folder.
Map<String, dynamic> mcpServerJson(
  McpServerDraft d, [
  Map<String, dynamic> base = const {},
]) {
  final out = Map<String, dynamic>.of(base);
  out.putIfAbsent('enabled', () => true);
  if (d.isUrl) {
    for (final k in ['command', 'args', 'env', 'env_from', 'cwd']) {
      out.remove(k);
    }
    out['url'] = d.url.trim();
    final headers = _pairs(d.headers) ?? const {};
    if (headers.isEmpty) {
      out.remove('headers_from');
    } else {
      out['headers_from'] = headers;
    }
  } else {
    for (final k in ['url', 'headers_from']) {
      out.remove(k);
    }
    out['command'] = d.command.trim();
    final args = mcpLines(d.args);
    if (args.isEmpty) {
      out.remove('args');
    } else {
      out['args'] = args;
    }
    final env = _pairs(d.env) ?? const {};
    if (env.isEmpty) {
      out.remove('env');
    } else {
      out['env'] = env;
    }
  }
  return out;
}

/// The device's server list, by name in list order, as saved.
typedef McpServers = Map<String, Map<String, dynamic>>;

/// [servers] with [server] saved as [name]: in place of [original] (same
/// place in the list, also when renamed), or added at the end when
/// [original] is null or gone. The other servers stay as they are.
McpServers mcpServersWithSaved(
  McpServers servers, {
  required String? original,
  required String name,
  required Map<String, dynamic> server,
}) {
  final next = <String, Map<String, dynamic>>{};
  for (final e in servers.entries) {
    next[e.key == original ? name : e.key] = e.key == original
        ? server
        : e.value;
  }
  next.putIfAbsent(name, () => server);
  return next;
}

/// [servers] with [name] turned on or off; its other keys stay.
McpServers mcpServersWithEnabled(McpServers servers, String name, bool on) => {
  for (final e in servers.entries)
    e.key: e.key == name
        ? (Map<String, dynamic>.of(e.value)..['enabled'] = on)
        : e.value,
};

/// [servers] without [name].
McpServers mcpServersWithout(McpServers servers, String name) => {
  for (final e in servers.entries)
    if (e.key != name) e.key: e.value,
};
