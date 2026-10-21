// Import checker for the add-on layer. Rules: wiki/architecture/addons.md.
//
//   dart scripts/check-addon-imports.dart
//   dart scripts/check-addon-imports.dart --self-test
//
// Standalone: no pub packages. A small lexer drops comments and reads string
// literals, so whitespace, line breaks, comments or string contents cannot hide
// or fake a directive. Only top-level (brace depth 0) import, export, part and
// part of directives count. Conditional directives check every candidate URI.

import 'dart:io';

/// Kit folders, relative to app/. They must not import or export any add-on.
const kitDirs = ['lib/ui/', 'lib/registry/', 'lib/services/'];

/// Engine code that add-ons may use.
const coreDirs = ['lib/core/', 'lib/state/', 'lib/src/'];

/// Existing app code reachable only by the temporary legacy adapter.
const legacyDirs = [
  'lib/blocks/',
  'lib/editor/',
  'lib/features/',
  'lib/properties/',
  'lib/views/',
];

void main(List<String> args) {
  if (args.contains('--self-test')) {
    exit(selfTest() ? 0 : 1);
  }
  final repo = File.fromUri(Platform.script).parent.parent;
  final result = checkApp(Directory('${repo.path}/app'));
  report(result);
  exit(result.violations.isEmpty ? 0 : 1);
}

/// Checks every .dart file under app/lib and app/test.
CheckResult checkApp(Directory app) {
  final path = app.path.replaceAll('\\', '/').replaceAll(RegExp(r'/+$'), '');
  final pubspec = File('$path/pubspec.yaml');
  if (!pubspec.existsSync()) throw StateError('no pubspec.yaml in $path');
  final name = RegExp(
    r'^name:\s*([A-Za-z0-9_]+)\s*$',
    multiLine: true,
  ).firstMatch(pubspec.readAsStringSync())?.group(1);
  if (name == null) throw StateError('pubspec.yaml has no name');
  return _Checker(path, name).run();
}

class Violation {
  Violation(this.file, this.line, this.rule, this.message);

  /// Relative to app/, with forward slashes.
  final String file;
  final int line;
  final String rule;
  final String message;

  @override
  String toString() => 'app/$file:$line: [$rule] $message';
}

class CheckResult {
  final List<Violation> violations = [];
  final List<String> warnings = [];
  int fileCount = 0;
  int addonCount = 0;
  int edgeCount = 0;
}

void report(CheckResult r) {
  final sorted = [...r.violations]
    ..sort(
      (a, b) => a.file != b.file
          ? a.file.compareTo(b.file)
          : a.line.compareTo(b.line),
    );
  for (final v in sorted) {
    stdout.writeln(v);
  }
  for (final w in r.warnings) {
    stdout.writeln('warning: $w');
  }
  if (r.violations.isEmpty) {
    stdout.writeln(
      'add-on imports OK: ${r.addonCount} add-ons, '
      '${r.fileCount} files, ${r.edgeCount} resolved directives',
    );
  } else {
    stdout.writeln('${r.violations.length} add-on import violation(s)');
  }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

enum Kind { ident, number, string, punct }

class Token {
  Token(this.kind, this.text, this.line, {this.interpolated = false});

  final Kind kind;
  final String text;
  final int line;

  /// True for a string literal with `$name` or `${...}` in it.
  final bool interpolated;

  bool isPunct(String p) => kind == Kind.punct && text == p;
  bool isWord(String w) => kind == Kind.ident && text == w;
}

class _Lexer {
  _Lexer(this.src);

  final String src;
  int pos = 0;
  int line = 1;

  List<Token> run() {
    if (src.startsWith('#!')) {
      final nl = src.indexOf('\n');
      pos = nl < 0 ? src.length : nl;
    }
    final out = <Token>[];
    _code(out, nested: false);
    return out;
  }

  String _at(int i) => i < src.length ? src[i] : '';

  void _moveTo(int to) {
    for (var i = pos; i < to && i < src.length; i++) {
      if (src.codeUnitAt(i) == 0x0a) line++;
    }
    pos = to;
  }

  /// Lexes code into [out]. With [nested], stops after the unmatched `}` of a `${...}`.
  void _code(List<Token> out, {required bool nested}) {
    var braces = 0;
    while (pos < src.length) {
      final ch = src[pos];
      final code = src.codeUnitAt(pos);
      if (ch == ' ' || ch == '\t' || ch == '\n' || ch == '\r') {
        _moveTo(pos + 1);
      } else if (ch == '/' && _at(pos + 1) == '/') {
        final end = src.indexOf('\n', pos);
        _moveTo(end < 0 ? src.length : end);
      } else if (ch == '/' && _at(pos + 1) == '*') {
        _skipBlockComment();
      } else if (ch == "'" || ch == '"') {
        out.add(_string(raw: false));
      } else if (_isIdentStart(code)) {
        final start = pos;
        while (pos < src.length && _isIdentPart(src.codeUnitAt(pos))) {
          pos++;
        }
        final word = src.substring(start, pos);
        if (word == 'r' && (_at(pos) == "'" || _at(pos) == '"')) {
          out.add(_string(raw: true));
        } else {
          out.add(Token(Kind.ident, word, line));
        }
      } else if (_isDigit(code)) {
        final start = pos;
        while (pos < src.length) {
          final c = src.codeUnitAt(pos);
          final dotDigit =
              src[pos] == '.' &&
              pos + 1 < src.length &&
              _isDigit(src.codeUnitAt(pos + 1));
          if (!_isIdentPart(c) && !dotDigit) break;
          pos++;
        }
        out.add(Token(Kind.number, src.substring(start, pos), line));
      } else {
        if (ch == '{') {
          braces++;
        } else if (ch == '}') {
          if (nested && braces == 0) {
            _moveTo(pos + 1);
            return;
          }
          braces--;
        }
        if (ch == '=' && _at(pos + 1) == '>') {
          out.add(Token(Kind.punct, '=>', line));
          _moveTo(pos + 2);
        } else {
          out.add(Token(Kind.punct, ch, line));
          _moveTo(pos + 1);
        }
      }
    }
  }

  Token _string({required bool raw}) {
    final startLine = line;
    final q = src[pos];
    final triple = src.startsWith(q * 3, pos);
    final delim = triple ? q * 3 : q;
    _moveTo(pos + delim.length);
    final buf = StringBuffer();
    var interpolated = false;
    while (pos < src.length) {
      final ch = src[pos];
      if (src.startsWith(delim, pos)) {
        _moveTo(pos + delim.length);
        return Token(
          Kind.string,
          buf.toString(),
          startLine,
          interpolated: interpolated,
        );
      }
      if (!triple && ch == '\n') break; // unterminated single-line string
      if (!raw && ch == '\\') {
        buf.write(
          src.substring(pos, pos + 2 > src.length ? src.length : pos + 2),
        );
        _moveTo(pos + 2);
        continue;
      }
      if (!raw && ch == '\$') {
        final next = _at(pos + 1);
        if (next == '{') {
          interpolated = true;
          _moveTo(pos + 2);
          _code(<Token>[], nested: true);
          continue;
        }
        if (next.isNotEmpty &&
            next != '\$' &&
            _isIdentStart(next.codeUnitAt(0))) {
          interpolated = true;
          pos++;
          while (pos < src.length && _isIdentPart(src.codeUnitAt(pos))) {
            pos++;
          }
          continue;
        }
      }
      buf.write(ch);
      _moveTo(pos + 1);
    }
    return Token(
      Kind.string,
      buf.toString(),
      startLine,
      interpolated: interpolated,
    );
  }

  void _skipBlockComment() {
    var depth = 0;
    while (pos < src.length) {
      if (src.startsWith('/*', pos)) {
        depth++;
        _moveTo(pos + 2);
      } else if (src.startsWith('*/', pos)) {
        depth--;
        _moveTo(pos + 2);
        if (depth == 0) return;
      } else {
        _moveTo(pos + 1);
      }
    }
  }

  static bool _isIdentStart(int c) =>
      c == 0x5f ||
      c == 0x24 ||
      (c >= 0x41 && c <= 0x5a) ||
      (c >= 0x61 && c <= 0x7a) ||
      c > 0x7f;
  static bool _isIdentPart(int c) => _isIdentStart(c) || _isDigit(c);
  static bool _isDigit(int c) => c >= 0x30 && c <= 0x39;
}

// ---------------------------------------------------------------------------
// Directives
// ---------------------------------------------------------------------------

enum DirectiveKind { import, export, part, partOf }

class Directive {
  Directive(this.kind, this.uri, this.line);

  final DirectiveKind kind;

  /// Null when the directive has no string literal URI.
  final Token? uri;
  final int line;
}

/// Top-level directives in token order. Conditional directives yield one entry per candidate URI.
List<Directive> parseDirectives(List<Token> t) {
  final out = <Directive>[];
  var depth = 0;
  for (var i = 0; i < t.length; i++) {
    final tok = t[i];
    if (tok.isPunct('{')) {
      depth++;
      continue;
    }
    if (tok.isPunct('}')) {
      depth--;
      continue;
    }
    if (depth != 0 || tok.kind != Kind.ident) continue;
    if (tok.text == 'import' || tok.text == 'export') {
      final kind = tok.text == 'import'
          ? DirectiveKind.import
          : DirectiveKind.export;
      final uris = <Token>[];
      var paren = 0;
      var j = i + 1;
      for (; j < t.length && !t[j].isPunct(';'); j++) {
        if (t[j].isPunct('(')) paren++;
        if (t[j].isPunct(')')) paren--;
        // Strings inside `if (dart.library.x == 'y')` are conditions, not URIs.
        if (paren == 0 && t[j].kind == Kind.string) uris.add(t[j]);
      }
      if (uris.isEmpty) out.add(Directive(kind, null, tok.line));
      for (final u in uris) {
        out.add(Directive(kind, u, tok.line));
      }
      i = j;
    } else if (tok.text == 'part') {
      final next = i + 1 < t.length ? t[i + 1] : null;
      if (next != null && next.isWord('of')) {
        final lib = i + 2 < t.length ? t[i + 2] : null;
        // `part of name;` names a library, not a file, so there is nothing to resolve.
        if (lib != null && lib.kind == Kind.string) {
          out.add(Directive(DirectiveKind.partOf, lib, tok.line));
        }
      } else if (next != null && next.kind == Kind.string) {
        out.add(Directive(DirectiveKind.part, next, tok.line));
      }
    }
  }
  return out;
}

class SourceFile {
  SourceFile(this.rel, this.tokens, this.directives);

  /// Relative to app/, with forward slashes.
  final String rel;
  final List<Token> tokens;
  final List<Directive> directives;

  String get dir => rel.substring(0, rel.lastIndexOf('/'));
}

class Edge {
  Edge(this.from, this.directive, this.target);

  final SourceFile from;
  final Directive directive;
  final String target;
}

class AddonEntry {
  AddonEntry(this.id);

  final String id;
  List<String> dependsOn = const [];
}

// ---------------------------------------------------------------------------
// Checker
// ---------------------------------------------------------------------------

String? addonOf(String rel) {
  for (final root in const ['lib/addons/', 'test/addons/']) {
    if (rel.startsWith(root)) {
      final rest = rel.substring(root.length);
      final slash = rest.indexOf('/');
      return slash < 0 ? null : rest.substring(0, slash);
    }
  }
  return null;
}

String _name(String path) => path.replaceAll('\\', '/').split('/').last;

String _apiPath(String id) => 'lib/addons/$id/api.dart';

class _Checker {
  _Checker(this.appPath, this.pkg);

  final String appPath;
  final String pkg;
  final CheckResult result = CheckResult();
  final Map<String, SourceFile> files = {};
  final Map<String, AddonEntry> entries = {};
  final List<Edge> edges = [];
  final Map<String, Set<String>> usedDeps = {};

  void add(String file, int line, String rule, String message) {
    result.violations.add(Violation(file, line, rule, message));
  }

  CheckResult run() {
    for (final dir in const ['lib', 'test']) {
      final d = Directory('$appPath/$dir');
      if (!d.existsSync()) continue;
      for (final e in d.listSync(recursive: true)) {
        if (e is! File || !e.path.endsWith('.dart')) continue;
        final rel = e.path.replaceAll('\\', '/').substring(appPath.length + 1);
        final src = e.readAsStringSync();
        final tokens = _Lexer(src).run();
        files[rel] = SourceFile(rel, tokens, parseDirectives(tokens));
      }
    }
    result.fileCount = files.length;

    _loadAddonEntries();

    for (final f in files.values) {
      for (final d in f.directives) {
        final target = _resolve(f, d);
        if (target != null) edges.add(Edge(f, d, target));
      }
    }
    result.edgeCount = edges.length;

    for (final e in edges) {
      _checkEdge(e);
    }
    _checkParts();
    _checkLayout();
    _checkDependencyGraph();
    result.addonCount = entries.length;
    return result;
  }

  /// Resolves a directive to an app-relative lib/ or test/ path. Returns null for
  /// SDK and external packages, and for URIs that were reported as violations.
  String? _resolve(SourceFile from, Directive d) {
    final uri = d.uri;
    if (uri == null) {
      add(from.rel, d.line, 'directive', 'directive has no string URI');
      return null;
    }
    if (uri.interpolated) {
      add(
        from.rel,
        uri.line,
        'directive',
        'URI interpolation cannot be checked',
      );
      return null;
    }
    final text = uri.text;
    if (text.startsWith('dart:')) return null;

    final String joined;
    if (text.startsWith('package:')) {
      final prefix = 'package:$pkg/';
      if (!text.startsWith(prefix)) return null; // another package
      joined = 'lib/${text.substring(prefix.length)}';
    } else if (text.contains(':')) {
      add(from.rel, uri.line, 'directive', 'unsupported URI "$text"');
      return null;
    } else {
      joined = '${from.dir}/$text';
    }

    final target = _normalize(joined);
    if (target == null ||
        !(target.startsWith('lib/') || target.startsWith('test/')) ||
        !files.containsKey(target)) {
      add(from.rel, uri.line, 'unresolved-import', 'cannot resolve "$text"');
      return null;
    }
    return target;
  }

  static String? _normalize(String path) {
    final out = <String>[];
    for (final seg in path.split('/')) {
      if (seg.isEmpty || seg == '.') continue;
      if (seg == '..') {
        if (out.isEmpty) return null;
        out.removeLast();
      } else {
        out.add(seg);
      }
    }
    return out.join('/');
  }

  void _checkEdge(Edge e) {
    final from = e.from.rel;
    final target = e.target;
    final line = e.directive.line;
    final fromOwner = addonOf(from);
    final targetOwner = addonOf(target);

    if (kitDirs.any((d) => from.startsWith(d))) {
      if (targetOwner != null) {
        add(
          from,
          line,
          'kit-imports-addon',
          'kit must not import or export add-on "$targetOwner" ($target)',
        );
      }
      return;
    }

    if (fromOwner == null || targetOwner == fromOwner) return;

    if (targetOwner != null) {
      if (target != _apiPath(targetOwner)) {
        add(
          from,
          line,
          'private-access',
          'add-on "$fromOwner" reaches into $target; only ${_apiPath(targetOwner)} is public',
        );
        return;
      }
      final declared = entries[fromOwner]?.dependsOn ?? const <String>[];
      if (!declared.contains(targetOwner)) {
        add(
          from,
          line,
          'undeclared-dependency',
          'add-on "$fromOwner" uses "$targetOwner" but does not list it in dependsOn',
        );
      } else {
        usedDeps.putIfAbsent(fromOwner, () => <String>{}).add(targetOwner);
      }
      return;
    }

    if (from.startsWith('test/') && target.startsWith('test/')) return;
    final allowed = [
      ...kitDirs,
      ...coreDirs,
      if (fromOwner == 'legacy') ...legacyDirs,
    ];
    if (allowed.any((p) => target.startsWith(p))) return;
    add(
      from,
      line,
      'addon-forbidden-import',
      'add-on "$fromOwner" may not import $target (allowed: kit, core/state, '
          'its own folder, and dependsOn api.dart files; only the legacy '
          'adapter may import the remaining old feature folders)',
    );
  }

  void _checkParts() {
    for (final e in edges) {
      if (e.directive.kind == DirectiveKind.part) {
        final ok = edges.any(
          (o) =>
              o.from.rel == e.target &&
              o.directive.kind == DirectiveKind.partOf &&
              o.target == e.from.rel,
        );
        if (!ok) {
          add(
            e.from.rel,
            e.directive.line,
            'part-relation',
            '${e.target} must declare "part of" this library',
          );
        }
      } else if (e.directive.kind == DirectiveKind.partOf) {
        final ok = edges.any(
          (o) =>
              o.from.rel == e.target &&
              o.directive.kind == DirectiveKind.part &&
              o.target == e.from.rel,
        );
        if (!ok) {
          add(
            e.from.rel,
            e.directive.line,
            'part-relation',
            '${e.target} must list this file with "part"',
          );
        }
      }
    }
  }

  /// Loose files in lib/addons/ belong to no add-on. Loose files in test/addons/
  /// are kit-level tests (such as the registry's boot-order test) and are not checked.
  void _checkLayout() {
    for (final rel in files.keys) {
      if (rel.startsWith('lib/addons/') && addonOf(rel) == null) {
        add(rel, 1, 'layout', 'add-on code must live in lib/addons/<id>/');
      }
    }
  }

  void _loadAddonEntries() {
    final ids = <String>{};
    final libAddons = Directory('$appPath/lib/addons');
    if (libAddons.existsSync()) {
      for (final e in libAddons.listSync()) {
        if (e is Directory) ids.add(_name(e.path));
      }
    }
    for (final rel in files.keys) {
      final id = addonOf(rel);
      if (id != null && rel.startsWith('test/addons/') && !ids.contains(id)) {
        add(
          rel,
          1,
          'addon-entry',
          'test code for add-on "$id", which has no lib/addons/$id/ folder',
        );
      }
    }
    for (final id in ids.toList()..sort()) {
      _readEntry(id);
    }
  }

  void _readEntry(String id) {
    final api = _apiPath(id);
    final entry = AddonEntry(id);
    entries[id] = entry;
    final f = files[api];
    if (f == null) {
      add('lib/addons/$id', 0, 'addon-entry', 'add-on has no api.dart entry');
      return;
    }

    final t = f.tokens;
    final classes = <int>[];
    for (var i = 0; i + 3 < t.length; i++) {
      if (t[i].isWord('class') &&
          t[i + 1].kind == Kind.ident &&
          t[i + 2].isWord('extends') &&
          t[i + 3].isWord('Addon')) {
        classes.add(i);
      }
    }
    if (classes.length != 1) {
      add(
        api,
        1,
        'addon-entry',
        'expected exactly one class extending Addon, found ${classes.length}',
      );
      return;
    }

    // Getters declared directly in the class body (brace depth 1).
    final cls = classes.single;
    final getters = <String, int>{};
    var depth = 0;
    var opened = false;
    for (var j = cls; j < t.length; j++) {
      final x = t[j];
      if (x.isPunct('{')) {
        depth++;
        opened = true;
        continue;
      }
      if (x.isPunct('}')) {
        depth--;
        if (opened && depth == 0) break;
        continue;
      }
      if (opened &&
          depth == 1 &&
          x.isWord('get') &&
          j + 1 < t.length &&
          t[j + 1].kind == Kind.ident) {
        getters.putIfAbsent(t[j + 1].text, () => j);
      }
    }

    final idAt = getters['id'];
    final idValue = idAt == null ? null : _getterValue(t, idAt);
    if (idValue == null ||
        idValue.length != 1 ||
        idValue.single.kind != Kind.string ||
        idValue.single.interpolated) {
      add(
        api,
        t[cls].line,
        'addon-entry',
        'get id must return a single string literal',
      );
    } else if (idValue.single.text != id) {
      add(
        api,
        idValue.single.line,
        'addon-entry',
        'get id is "${idValue.single.text}" but the folder is "$id"',
      );
    }

    final depsAt = getters['dependsOn'];
    final deps = depsAt == null
        ? null
        : _literalStrings(_getterValue(t, depsAt));
    if (deps == null) {
      add(
        api,
        t[cls].line,
        'addon-entry',
        'get dependsOn must return a literal list of strings, such as const [\'board\'] or const <String>[]',
      );
    } else {
      entry.dependsOn = deps;
    }
  }

  /// Tokens of a getter's value: after `=>`, or after `{ return` in block form.
  List<Token>? _getterValue(List<Token> t, int getIndex) {
    final k = getIndex + 2;
    if (k >= t.length) return null;
    final int start;
    if (t[k].isPunct('=>')) {
      start = k + 1;
    } else if (t[k].isPunct('{') &&
        k + 1 < t.length &&
        t[k + 1].isWord('return')) {
      start = k + 2;
    } else {
      return null;
    }
    final out = <Token>[];
    for (var j = start; j < t.length; j++) {
      if (t[j].isPunct(';') || t[j].isPunct('}')) break;
      out.add(t[j]);
    }
    return out;
  }

  /// Accepts `const <String>['a', 'b']`, `['a']` and `const []`. Anything else is null.
  static List<String>? _literalStrings(List<Token>? v) {
    if (v == null) return null;
    var i = 0;
    if (i < v.length && v[i].isWord('const')) i++;
    if (i < v.length && v[i].isPunct('<')) {
      while (i < v.length && !v[i].isPunct('>')) {
        i++;
      }
      i++;
    }
    if (i >= v.length || !v[i].isPunct('[')) return null;
    i++;
    final out = <String>[];
    var expectItem = true;
    for (; i < v.length; i++) {
      final x = v[i];
      if (x.isPunct(']')) return i == v.length - 1 ? out : null;
      if (expectItem) {
        if (x.kind != Kind.string || x.interpolated) return null;
        out.add(x.text);
        expectItem = false;
      } else {
        if (!x.isPunct(',')) return null;
        expectItem = true;
      }
    }
    return null;
  }

  void _checkDependencyGraph() {
    final ids = entries.keys.toList()..sort();
    for (final id in ids) {
      final e = entries[id]!;
      for (final dep in e.dependsOn) {
        if (dep == id) {
          add(
            _apiPath(id),
            0,
            'dependency-cycle',
            'add-on "$id" lists itself in dependsOn',
          );
        } else if (!entries.containsKey(dep)) {
          add(
            _apiPath(id),
            0,
            'missing-dependency',
            'dependsOn "$dep" has no lib/addons/$dep/api.dart',
          );
        }
      }
      for (final dep in e.dependsOn) {
        if (dep != id && !(usedDeps[id]?.contains(dep) ?? false)) {
          result.warnings.add(
            'add-on "$id" declares dependsOn "$dep" but never imports its api.dart',
          );
        }
      }
    }

    final state = <String, int>{}; // 1 = on the stack, 2 = done
    void visit(String id, List<String> stack) {
      state[id] = 1;
      for (final dep in entries[id]!.dependsOn) {
        if (dep == id || !entries.containsKey(dep)) continue;
        if (state[dep] == 1) {
          add(
            _apiPath(id),
            0,
            'dependency-cycle',
            'dependsOn cycle: ${[...stack, id, dep].join(' -> ')}',
          );
        } else if (state[dep] == null) {
          visit(dep, [...stack, id]);
        }
      }
      state[id] = 2;
    }

    for (final id in ids) {
      if (state[id] == null) visit(id, []);
    }
  }
}

// ---------------------------------------------------------------------------
// Self-test: a clean fixture, then one planted violation per case.
// ---------------------------------------------------------------------------

class _Case {
  _Case(this.name, this.edit, this.expect);

  final String name;
  final void Function(Map<String, String> files) edit;

  /// Rules that must appear. Empty means the fixture must pass with no violations.
  final Set<String> expect;
}

void _replace(Map<String, String> f, String path, String from, String to) {
  final src = f[path];
  if (src == null || !src.contains(from)) {
    throw StateError('fixture $path lacks "$from"');
  }
  f[path] = src.replaceFirst(from, to);
}

void _append(Map<String, String> f, String path, String text) {
  f[path] = '${f[path]}\n$text\n';
}

Map<String, String> _baseFixture() => {
  'pubspec.yaml': 'name: notes_app\nversion: 1.0.0\n',
  'lib/main.dart': "import 'shell/shell.dart';\n\nvoid main() => shell();\n",
  'lib/shell/shell.dart':
      "import '../addons/board/api.dart';\n"
      "import '../addons/database/api.dart';\n"
      "import '../addons/document/api.dart';\n"
      "import '../addons/legacy/api.dart';\n\nvoid shell() {}\n",
  'lib/registry/addon.dart':
      'abstract class Addon {\n  const Addon();\n  String get id;\n'
      '  List<String> get dependsOn => const [];\n}\n',
  'lib/registry/registry.dart': "import 'addon.dart';\nexport 'addon.dart';\n",
  'lib/ui/ui.dart': "import '../state/state.dart';\n\nclass Ui {}\n",
  'lib/services/services.dart':
      "import '../core/core.dart';\n\nclass Services {}\n",
  'lib/core/core.dart': 'class Core {}\n',
  'lib/state/state.dart': "import '../core/core.dart';\n\nclass AppState {}\n",
  'lib/features/note.dart': "import '../ui/ui.dart';\n\nclass Note {}\n",
  'lib/addons/database/api.dart': r'''
import 'package:notes_app/registry/addon.dart';
import 'package:notes_app/core/core.dart';

export 'src/store.dart';

class DatabaseAddon extends Addon {
  const DatabaseAddon();

  @override
  String get id => 'database';

  @override
  List<String> get dependsOn => const <String>[];
}
''',
  'lib/addons/database/src/store.dart': r'''
import 'package:notes_app/core/core.dart';

part 'store_part.dart';

class Store {}
''',
  'lib/addons/database/src/store_part.dart': r'''
part of 'store.dart';

extension StorePart on Store {}
''',
  'lib/addons/board/api.dart': r'''
import 'package:notes_app/registry/addon.dart';
import '../database/api.dart';

export 'src/board.dart';

class BoardAddon extends Addon {
  const BoardAddon();

  @override
  String get id => 'board';

  @override
  List<String> get dependsOn => const ['database'];
}
''',
  'lib/addons/board/src/board.dart': r'''
import 'package:notes_app/addons/database/api.dart';
import 'package:notes_app/ui/ui.dart';
import 'board_stub.dart' if (dart.library.io) 'board_io.dart';

part 'board_part.dart';

// import 'package:notes_app/addons/database/src/store.dart';
/* export '../../database/src/store.dart'; */

final String note = "import 'package:notes_app/addons/database/src/store.dart';";
final String tricky = "a ${'}'} import '../../database/src/store.dart';";

class Board {}
''',
  'lib/addons/board/src/board_part.dart': "part of 'board.dart';\n",
  'lib/addons/board/src/board_stub.dart': 'class BoardIo {}\n',
  'lib/addons/board/src/board_io.dart': 'class BoardIo {}\n',
  'lib/addons/document/api.dart': r'''
import 'package:notes_app/registry/addon.dart';

export 'src/doc.dart';

class DocumentAddon extends Addon {
  const DocumentAddon();

  @override
  String get id => 'document';

  @override
  List<String> get dependsOn => const ['board'];
}
''',
  'lib/addons/document/src/doc.dart': r'''
import 'package:notes_app/addons/board/api.dart';
import '../../../core/core.dart';

class Doc {}
''',
  'lib/addons/legacy/api.dart': r'''
import 'package:notes_app/registry/addon.dart';
import '../../features/note.dart';

class LegacyAddon extends Addon {
  const LegacyAddon();

  @override
  String get id => 'legacy';

  @override
  List<String> get dependsOn => const <String>[];
}
''',
  'test/helpers.dart': 'void helper() {}\n',
  'test/addons/registry_test.dart': r'''
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/registry/addon.dart';

void main() {}
''',
  'test/addons/board/board_test.dart': r'''
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/board/api.dart';
import 'package:notes_app/addons/board/src/board.dart';
import '../../helpers.dart';

void main() {}
''',
};

void _writeFixture(Directory root, Map<String, String> files) {
  for (final entry in files.entries) {
    final file = File('${root.path}/app/${entry.key}');
    file.createSync(recursive: true);
    file.writeAsStringSync(entry.value);
  }
}

bool selfTest() {
  final tmp = Directory.systemTemp.createTempSync('addon-import-check-');
  var failures = 0;
  try {
    final cases = <_Case>[
      _Case('clean fixture passes', (f) {}, const {}),
      _Case(
        'a migrated add-on cannot bypass the host facade through old features',
        (f) {
          _append(
            f,
            'lib/addons/database/api.dart',
            "import '../../features/note.dart';",
          );
        },
        const {'addon-forbidden-import'},
      ),
      _Case('private package URI', (f) {
        _append(
          f,
          'lib/addons/board/src/board.dart',
          "import 'package:notes_app/addons/database/src/store.dart';",
        );
      }, const {'private-access'}),
      _Case('private relative path', (f) {
        _append(
          f,
          'lib/addons/document/src/doc.dart',
          "import '../../database/src/store.dart';",
        );
      }, const {'private-access'}),
      _Case('whitespace and comments around a private import', (f) {
        _append(
          f,
          'lib/addons/board/src/board.dart',
          "import\n  /* a */ 'package:notes_app/addons/database/src/store.dart' // b\n  ;",
        );
      }, const {'private-access'}),
      _Case('private re-export from api.dart', (f) {
        _append(
          f,
          'lib/addons/board/api.dart',
          "export 'package:notes_app/addons/database/src/store.dart';",
        );
      }, const {'private-access'}),
      _Case('private URI as the conditional candidate', (f) {
        _append(
          f,
          'lib/addons/board/src/board.dart',
          "import 'board_stub.dart' if (dart.library.io) "
              "'package:notes_app/addons/database/src/store.dart';",
        );
      }, const {'private-access'}),
      _Case('part file from another add-on', (f) {
        _append(
          f,
          'lib/addons/board/src/board.dart',
          "part 'package:notes_app/addons/database/src/store_part.dart';",
        );
      }, const {'private-access', 'part-relation'}),
      _Case('private access from test code', (f) {
        _append(
          f,
          'test/addons/board/board_test.dart',
          "import 'package:notes_app/addons/database/src/store.dart';",
        );
      }, const {'private-access'}),
      _Case('undeclared dependency on another api', (f) {
        _append(
          f,
          'lib/addons/legacy/api.dart',
          "import 'package:notes_app/addons/database/api.dart';",
        );
      }, const {'undeclared-dependency'}),
      _Case('undeclared dependency from test code', (f) {
        _append(
          f,
          'test/addons/board/board_test.dart',
          "import 'package:notes_app/addons/document/api.dart';",
        );
      }, const {'undeclared-dependency'}),
      _Case('declared dependency with no add-on folder', (f) {
        _replace(
          f,
          'lib/addons/board/api.dart',
          "const ['database']",
          "const ['database', 'ghost']",
        );
      }, const {'missing-dependency'}),
      _Case('dependsOn is not a literal list', (f) {
        _replace(f, 'lib/addons/board/api.dart', "const ['database']", '_deps');
      }, const {'addon-entry'}),
      _Case('id does not match the folder', (f) {
        _replace(
          f,
          'lib/addons/database/api.dart',
          "=> 'database';",
          "=> 'db';",
        );
      }, const {'addon-entry'}),
      _Case('test folder without an add-on', (f) {
        f['test/addons/ghost/ghost_test.dart'] = 'void main() {}\n';
      }, const {'addon-entry'}),
      _Case('add-on without api.dart', (f) {
        f.remove('lib/addons/document/api.dart');
      }, const {'addon-entry'}),
      _Case('dependsOn cycle', (f) {
        _replace(
          f,
          'lib/addons/database/api.dart',
          'const <String>[]',
          "const ['board']",
        );
      }, const {'dependency-cycle'}),
      _Case('kit ui imports an add-on', (f) {
        _append(f, 'lib/ui/ui.dart', "import '../addons/board/api.dart';");
      }, const {'kit-imports-addon'}),
      _Case('kit services exports a private add-on file', (f) {
        _append(
          f,
          'lib/services/services.dart',
          "export '../addons/database/src/store.dart';",
        );
      }, const {'kit-imports-addon'}),
      _Case('add-on imports the shell', (f) {
        _append(
          f,
          'lib/addons/document/src/doc.dart',
          "import '../../../shell/shell.dart';",
        );
      }, const {'addon-forbidden-import'}),
      _Case('unresolved relative import', (f) {
        _append(f, 'lib/addons/board/src/board.dart', "import 'missing.dart';");
      }, const {'unresolved-import'}),
      _Case('relative import escapes the root', (f) {
        _append(
          f,
          'lib/addons/board/src/board.dart',
          "import '../../../../../../etc/passwd.dart';",
        );
      }, const {'unresolved-import'}),
      _Case('part without a matching part of', (f) {
        _append(f, 'lib/addons/board/src/board.dart', "part 'board_io.dart';");
      }, const {'part-relation'}),
      _Case('loose file in lib/addons', (f) {
        f['lib/addons/readme.dart'] = 'void x() {}\n';
      }, const {'layout'}),
      _Case('malformed import', (f) {
        _append(f, 'lib/addons/board/src/board.dart', 'import ;');
      }, const {'directive'}),
      _Case('interpolated URI', (f) {
        _append(
          f,
          'lib/addons/board/src/board.dart',
          r'''import "package:notes_app/addons/${'database'}/api.dart";''',
        );
      }, const {'directive'}),
    ];

    for (final c in cases) {
      final files = _baseFixture();
      c.edit(files);
      final slug = c.name.replaceAll(RegExp(r'[^A-Za-z0-9]+'), '-');
      final root = Directory('${tmp.path}/$slug');
      _writeFixture(root, files);
      final result = checkApp(Directory('${root.path}/app'));
      final rules = result.violations.map((v) => v.rule).toSet();
      final missing = c.expect.where((r) => !rules.contains(r)).toList();
      final pass = c.expect.isEmpty
          ? result.violations.isEmpty
          : missing.isEmpty;
      if (pass) {
        stdout.writeln('PASS  ${c.name}');
      } else {
        failures++;
        stdout.writeln(
          'FAIL  ${c.name}: expected ${c.expect.toList()}, '
          'got ${rules.toList()}',
        );
        for (final v in result.violations) {
          stdout.writeln('      $v');
        }
      }
    }
  } finally {
    tmp.deleteSync(recursive: true);
  }
  stdout.writeln(
    failures == 0 ? 'self-test passed' : 'self-test: $failures case(s) failed',
  );
  return failures == 0;
}
