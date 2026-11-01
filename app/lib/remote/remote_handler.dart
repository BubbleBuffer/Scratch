// The request side of the app remote-control endpoint
// (wiki/integrations/app-remote.md), free of sockets and widgets: a request
// goes in as method, path, Authorization header and body text, a status and
// a JSON map come out. What the commands do to the app is behind
// [RemoteBackend], so the handler is tested with a fake.

import 'dart:convert';

/// A failure the handler reports as `{"ok": false, "error": message}` with
/// [status] (400 bad arguments, 404 unknown page or element).
class RemoteError implements Exception {
  const RemoteError(this.status, this.message);
  final int status;
  final String message;

  @override
  String toString() => 'RemoteError($status): $message';
}

/// A PNG of the app's frame.
class RemoteImage {
  const RemoteImage({
    required this.width,
    required this.height,
    required this.png,
  });
  final int width;
  final int height;
  final List<int> png;
}

/// A parsed key combination: modifiers from [keyModifiers] and one key name
/// accepted by [isKeyName].
class KeyCombo {
  const KeyCombo(this.modifiers, this.key);
  final Set<String> modifiers;
  final String key;

  @override
  String toString() => [...modifiers, key].join('+');
}

const keyModifiers = ['ctrl', 'shift', 'alt', 'meta'];

const _namedKeys = {
  'enter',
  'escape',
  'tab',
  'up',
  'down',
  'left',
  'right',
  'backspace',
  'delete',
  'space',
};

/// Whether [name] is a key the `key` command accepts: a named key, a letter,
/// a digit or `f1`..`f12`.
bool isKeyName(String name) {
  if (_namedKeys.contains(name)) return true;
  if (RegExp(r'^[a-z0-9]$').hasMatch(name)) return true;
  final f = RegExp(r'^f([0-9]{1,2})$').firstMatch(name);
  if (f == null) return false;
  final n = int.parse(f.group(1)!);
  return n >= 1 && n <= 12;
}

/// Parses `ctrl+k`, `shift+tab`, `f5`. Throws a 400 [RemoteError].
KeyCombo parseKeyCombo(String combo) {
  final parts = combo.toLowerCase().split('+').map((p) => p.trim()).toList();
  if (parts.any((p) => p.isEmpty)) {
    throw RemoteError(400, 'bad key combo "$combo"');
  }
  final key = parts.removeLast();
  final mods = <String>{};
  for (final p in parts) {
    if (!keyModifiers.contains(p)) {
      throw RemoteError(
        400,
        'unknown modifier "$p"; modifiers: ${keyModifiers.join(', ')}',
      );
    }
    mods.add(p);
  }
  if (!isKeyName(key)) {
    throw RemoteError(
      400,
      'unknown key "$key"; keys: enter, escape, tab, up, down, left, right, '
      'backspace, delete, space, a-z, 0-9, f1-f12',
    );
  }
  return KeyCombo(mods, key);
}

/// What the commands do to the running app.
abstract interface class RemoteBackend {
  /// `vault` (`{"open", "name"}`) and `page` (`{"id","title"}` or null).
  Future<Map<String, Object?>> status();

  /// Opens the page with id or exact title [page]; returns `{"id","title"}`.
  /// Throws a 404 [RemoteError] when none matches.
  Future<Map<String, Object?>> open(String page);

  /// [route] is one of `home`, `settings`, `search`, `back`, `page/<id>`.
  Future<void> navigate(String route);

  Future<RemoteImage> screenshot(double pixelRatio);

  /// The `nodes` of the `tree` command.
  Future<List<Map<String, Object?>>> tree();

  /// 404 [RemoteError] when [id] is not in the current tree.
  Future<void> tap(String id);

  /// Runs the node's focus action, waits a frame and returns whether the node
  /// now reports focus. 404 when [id] is not in the current tree, 400 when it
  /// has no focus action.
  Future<bool> focus(String id);

  Future<void> type(String id, String text, {required bool replace});

  Future<void> key(KeyCombo combo);
}

class RemoteReply {
  const RemoteReply(this.status, this.body);
  final int status;
  final Map<String, Object?> body;
}

const navigateRoutes = ['home', 'settings', 'search', 'back', 'page/<id>'];

class RemoteHandler {
  RemoteHandler({
    required this.backend,
    required this.token,
    required this.version,
    required this.pid,
  });

  final RemoteBackend backend;
  final String token;
  final String version;
  final int pid;

  /// Whether the Authorization header value carries [token].
  bool authorized(String? header) {
    if (header == null) return false;
    final m = RegExp(
      r'^bearer\s+(\S+)\s*$',
      caseSensitive: false,
    ).firstMatch(header);
    if (m == null) return false;
    return constantTimeEquals(m.group(1)!, token);
  }

  Future<RemoteReply> handle({
    required String method,
    required String path,
    required String? authorization,
    required String body,
  }) async {
    if (method != 'POST') return _fail(405, 'use POST');
    if (!authorized(authorization)) {
      return _fail(401, 'missing or wrong token');
    }
    const prefix = '/v1/';
    if (!path.startsWith(prefix)) return _fail(404, 'unknown path $path');
    final command = path.substring(prefix.length);
    if (!_commands.contains(command)) {
      return _fail(404, 'unknown command "$command"');
    }
    try {
      final args = _parseBody(body);
      final fields = await _run(command, args);
      return RemoteReply(200, {'ok': true, ...fields});
    } on RemoteError catch (e) {
      return _fail(e.status, e.message);
    } catch (e) {
      return _fail(500, '$e');
    }
  }

  static const _commands = {
    'status',
    'open',
    'navigate',
    'screenshot',
    'tree',
    'tap',
    'focus',
    'type',
    'key',
  };

  RemoteReply _fail(int status, String message) =>
      RemoteReply(status, {'ok': false, 'error': message});

  Map<String, Object?> _parseBody(String text) {
    if (text.trim().isEmpty) return const {};
    final Object? decoded;
    try {
      decoded = jsonDecode(text);
    } on FormatException {
      throw const RemoteError(400, 'request body is not valid JSON');
    }
    if (decoded is! Map<String, Object?>) {
      throw const RemoteError(400, 'request body must be a JSON object');
    }
    return decoded;
  }

  String _string(Map<String, Object?> args, String name) {
    final v = args[name];
    if (v is! String || v.isEmpty) {
      throw RemoteError(400, '"$name" must be a non-empty string');
    }
    return v;
  }

  Future<Map<String, Object?>> _run(
    String command,
    Map<String, Object?> args,
  ) async {
    switch (command) {
      case 'status':
        final s = await backend.status();
        return {'version': version, 'pid': pid, ...s};
      case 'open':
        return {'page': await backend.open(_string(args, 'page'))};
      case 'navigate':
        final route = _string(args, 'route');
        final ok =
            const {'home', 'settings', 'search', 'back'}.contains(route) ||
            (route.startsWith('page/') && route.length > 5);
        if (!ok) {
          throw RemoteError(
            400,
            'unknown route "$route"; routes: ${navigateRoutes.join(', ')}',
          );
        }
        await backend.navigate(route);
        return {'route': route};
      case 'screenshot':
        final raw = args['pixel_ratio'];
        var ratio = 1.0;
        if (raw != null) {
          if (raw is! num || raw <= 0 || raw > 4) {
            throw const RemoteError(
              400,
              '"pixel_ratio" must be a number above 0 and at most 4',
            );
          }
          ratio = raw.toDouble();
        }
        final image = await backend.screenshot(ratio);
        return {
          'width': image.width,
          'height': image.height,
          'png_base64': base64Encode(image.png),
        };
      case 'tree':
        return {'nodes': await backend.tree()};
      case 'tap':
        await backend.tap(_string(args, 'id'));
        return const {};
      case 'focus':
        return {'focused': await backend.focus(_string(args, 'id'))};
      case 'type':
        final text = args['text'];
        if (text is! String) {
          throw const RemoteError(400, '"text" must be a string');
        }
        final replace = args['replace'];
        if (replace != null && replace is! bool) {
          throw const RemoteError(400, '"replace" must be true or false');
        }
        await backend.type(
          _string(args, 'id'),
          text,
          replace: (replace as bool?) ?? true,
        );
        return const {};
      case 'key':
        await backend.key(parseKeyCombo(_string(args, 'combo')));
        return const {};
    }
    throw RemoteError(404, 'unknown command "$command"');
  }
}

/// Compares two strings without stopping at the first difference.
bool constantTimeEquals(String a, String b) {
  final x = utf8.encode(a);
  final y = utf8.encode(b);
  var diff = x.length ^ y.length;
  final n = x.length > y.length ? x.length : y.length;
  for (var i = 0; i < n; i++) {
    final p = i < x.length ? x[i] : 0;
    final q = i < y.length ? y[i] : 0;
    diff |= p ^ q;
  }
  return diff == 0;
}
