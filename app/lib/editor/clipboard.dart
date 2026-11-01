// Blocks on the system clipboard as markdown: copying selected blocks
// writes markdown (children indented four spaces, inline marks as
// markdown), and pasting several lines parses them back into blocks. The
// format follows the core's markdown export closely enough that blocks
// survive a round trip through another app; kinds markdown cannot express
// (toggles, callouts) use small conventions of their own.

import 'dart:convert';

import 'package:flutter/services.dart';

import '../core/models.dart';
import '../registry/block_codecs.dart';
import '../registry/block_registry.dart';
import '../registry/embed_block.dart';
import 'model.dart';

// ---------------------------------------------------------------------------
// Blocks to markdown and plain text. The kind-specific lines come from each
// kind's codecs (see registry/block_codecs.dart); this walks the tree.

/// [tops] (and their children) as markdown: the page's markdown export.
String nodesToMarkdown(List<Node> tops, {BlockRegistry? registry}) {
  final reg = registry ?? blockRegistry;
  return _markdown(tops, (kind) => reg.codecsOf(kind).markdown);
}

/// Plain text of [tops] (one line per block, no markers).
String nodesToPlainText(List<Node> tops, {BlockRegistry? registry}) {
  final reg = registry ?? blockRegistry;
  final out = <String>[];
  void write(Node n) {
    final line = reg.codecsOf(n.kind).plainText.line(n);
    if (line.isNotEmpty) out.add(line);
    n.children.forEach(write);
  }

  tops.forEach(write);
  return out.join('\n');
}

String _markdown(List<Node> tops, MarkdownCodec Function(String kind) codecFor) {
  final out = <String>[];
  void write(Node n, int level) {
    final codec = codecFor(n.kind);
    if (codec.flattens) {
      // Columns flatten to their contents; a table of contents has none.
      for (final c in n.children) {
        write(c, level);
      }
      return;
    }
    final pad = '    ' * level;
    for (final l in codec.write(n, inlineToMarkdown(n.text, n.marks))) {
      out.add('$pad$l');
    }
    for (final c in n.children) {
      write(c, level + 1);
    }
  }

  for (final n in tops) {
    write(n, 0);
  }
  return out.join('\n');
}

/// [text] with [marks] as inline markdown.
String inlineToMarkdown(String text, List<BMark> marks) {
  if (marks.isEmpty) return text;
  final opens = <int, List<String>>{}, closes = <int, List<String>>{};
  for (final m in marks) {
    final (open, close) = switch (m.kind) {
      BMarkKind.bold => ('**', '**'),
      BMarkKind.italic => ('*', '*'),
      BMarkKind.strike => ('~~', '~~'),
      BMarkKind.code => ('`', '`'),
      BMarkKind.link => ('[', '](${m.url})'),
      BMarkKind.pageLink => ('[', '](page:${m.url})'),
      _ => ('', ''),
    };
    if (open.isEmpty) continue;
    (opens[m.start] ??= []).add(open);
    (closes[m.end] ??= []).insert(0, close);
  }
  final b = StringBuffer();
  for (var i = 0; i <= text.length; i++) {
    for (final c in closes[i] ?? const <String>[]) {
      b.write(c);
    }
    for (final o in opens[i] ?? const <String>[]) {
      b.write(o);
    }
    if (i < text.length) b.write(text[i]);
  }
  return b.toString();
}

// ---------------------------------------------------------------------------
// Markdown to blocks.

/// Blocks (with children) parsed from [md]. Plain lines become paragraphs.
List<Node> markdownToNodes(String md) {
  final lines = md.replaceAll('\r\n', '\n').replaceAll('\r', '\n').split('\n');
  final roots = <Node>[];
  // (indent width, node) of the open ancestors.
  final stack = <(int, Node)>[];
  var i = 0;
  while (i < lines.length) {
    final raw = lines[i];
    final expanded = raw.replaceAll('\t', '    ');
    final body = expanded.trimLeft();
    final indent = expanded.length - body.length;
    i++;
    if (body.isEmpty) continue;
    Node n;
    if (body.startsWith('```')) {
      final code = <String>[];
      while (i < lines.length && !lines[i].trimLeft().startsWith('```')) {
        final l = lines[i].replaceAll('\t', '    ');
        code.add(l.length >= indent && l.substring(0, indent).trim().isEmpty ? l.substring(indent) : l.trimLeft());
        i++;
      }
      i++; // closing fence
      n = Node(id: newBlockId(), kind: Kinds.code, text: code.join('\n'));
    } else {
      final annotation = _embedAnnotation(body);
      if (annotation == null) {
        n = _lineToNode(body);
      } else {
        n = annotation.$1;
        // Fallback lines are display text, never the embed's stored title.
        i = (i + annotation.$2 - 1).clamp(0, lines.length);
      }
    }
    while (stack.isNotEmpty && stack.last.$1 >= indent) {
      stack.removeLast();
    }
    if (n.kind == Kinds.numbered) {
      // A run's first item keeps its typed number when that isn't one; the
      // items after it continue the run and store nothing.
      final typed = int.tryParse(_numbered.firstMatch(body)?.group(1) ?? '');
      final siblings = stack.isEmpty ? roots : stack.last.$2.children;
      final startsRun = siblings.isEmpty || siblings.last.kind != Kinds.numbered;
      if (typed != null && typed != 1 && typed <= maxNumberedStart && startsRun) {
        n.numberedStart = typed;
      }
    }
    if (stack.isEmpty) {
      roots.add(n);
    } else {
      n.parent = stack.last.$2;
      stack.last.$2.children.add(n);
    }
    stack.add((indent, n));
  }
  return roots;
}

final _numbered = RegExp(r'^(\d+)[.)] ');
final _media = RegExp(r'^(!?)\[([^\]]*)\]\(([^)]*)\)$');

/// Reads the core's annotated embed fallback without copying its title into
/// the block. Invalid annotations retain their ordinary Markdown meaning.
(Node, int)? _embedAnnotation(String line) {
  if (!line.startsWith('>')) return null;
  final match = RegExp(r' <!-- ([^<>]+) -->\s*$').firstMatch(line);
  if (match == null) return null;
  String? encoded, color;
  int? width;
  for (final word in match.group(1)!.split(RegExp(r'\s+'))) {
    final split = word.indexOf('=');
    if (split < 0) return null;
    final key = word.substring(0, split), value = word.substring(split + 1);
    switch (key) {
      case 'block':
        if (encoded != null) return null;
        encoded = value;
      case 'color':
        if (value.isEmpty) return null;
        color = value;
      case 'width':
        width = int.tryParse(value);
        if (width == null || width < 1 || width >= 100) return null;
      default:
        return null;
    }
  }
  if (encoded == null ||
      encoded.length.isOdd ||
      !RegExp(r'^[0-9a-f]+$').hasMatch(encoded)) {
    return null;
  }
  try {
    final bytes = [
      for (var j = 0; j < encoded.length; j += 2)
        int.parse(encoded.substring(j, j + 2), radix: 16),
    ];
    final payload = jsonDecode(utf8.decode(bytes));
    if (payload is! Map<String, dynamic> ||
        payload['kind'] != embedKind ||
        payload['lines'] is! int ||
        (payload['lines'] as int) < 1 ||
        payload['checked'] is! bool ||
        (payload['target'] != null && payload['target'] is! String)) {
      return null;
    }
    return (
      Node(
        id: newBlockId(),
        kind: embedKind,
        target: payload['target'] as String?,
        checked: payload['checked'] as bool,
        color: color,
        width: width,
      ),
      payload['lines'] as int,
    );
  } catch (_) {
    return null;
  }
}

Node _lineToNode(String s) {
  String kind = Kinds.paragraph;
  var rest = s;
  var checked = false;
  String? target;
  if (s == '---' || s == '***' || s == '___') {
    return Node(id: newBlockId(), kind: Kinds.divider);
  }
  final media = _media.firstMatch(s);
  if (media != null) {
    final img = media.group(1) == '!';
    final label = media.group(2)!, url = media.group(3)!;
    if (url.startsWith('attachment:')) {
      return Node(id: newBlockId(), kind: img ? Kinds.image : Kinds.file, text: img || label != 'File' ? label : '', target: url.substring(11));
    }
    if (url.startsWith('page:')) {
      return Node(id: newBlockId(), kind: Kinds.linkToPage, target: url.substring(5));
    }
    if (!img && label == url) return Node(id: newBlockId(), kind: Kinds.bookmark, target: url);
  }
  for (final (p, k) in const [
    ('### ', Kinds.heading3),
    ('## ', Kinds.heading2),
    ('# ', Kinds.heading1),
    ('- [ ] ', Kinds.todo),
    ('- [x] ', Kinds.todo),
    ('- [X] ', Kinds.todo),
    ('* [ ] ', Kinds.todo),
    ('* [x] ', Kinds.todo),
    ('- ', Kinds.bulleted),
    ('* ', Kinds.bulleted),
    ('+ ', Kinds.bulleted),
    ('• ', Kinds.bulleted),
    ('▸ ', Kinds.toggle),
    ('> ', Kinds.quote),
  ]) {
    if (s.startsWith(p)) {
      kind = k;
      rest = s.substring(p.length);
      checked = p.contains('x') || p.contains('X');
      break;
    }
  }
  if (kind == Kinds.paragraph) {
    final m = _numbered.firstMatch(s);
    if (m != null) {
      kind = Kinds.numbered;
      rest = s.substring(m.end);
    }
  }
  final (text, marks) = parseInline(rest);
  return Node(id: newBlockId(), kind: kind, text: text, marks: marks, checked: checked, target: target);
}

/// Inline markdown to text and marks: `**bold**`, `*italic*`/`_italic_`,
/// `~~strike~~`, `` `code` `` and `[text](url)` (`page:` links become page
/// mentions).
(String, List<BMark>) parseInline(String s) {
  final b = StringBuffer();
  final marks = <BMark>[];
  final open = <String, int>{};
  var i = 0;
  bool at(String t) => s.startsWith(t, i);
  while (i < s.length) {
    if (at('\\') && i + 1 < s.length) {
      b.write(s[i + 1]);
      i += 2;
      continue;
    }
    if (at('`')) {
      final end = s.indexOf('`', i + 1);
      if (end > i) {
        final start = b.length;
        b.write(s.substring(i + 1, end));
        marks.add(BMark(start: start, end: b.length, kind: BMarkKind.code, url: ''));
        i = end + 1;
        continue;
      }
    }
    if (at('[')) {
      final close = s.indexOf('](', i + 1);
      final end = close < 0 ? -1 : s.indexOf(')', close + 2);
      if (close > i && end > close) {
        final (inner, innerMarks) = parseInline(s.substring(i + 1, close));
        final url = s.substring(close + 2, end);
        final start = b.length;
        b.write(inner);
        for (final m in innerMarks) {
          marks.add(BMark(start: m.start + start, end: m.end + start, kind: m.kind, url: m.url));
        }
        marks.add(url.startsWith('page:')
            ? BMark(start: start, end: b.length, kind: BMarkKind.pageLink, url: url.substring(5))
            : BMark(start: start, end: b.length, kind: BMarkKind.link, url: url));
        i = end + 1;
        continue;
      }
    }
    String? tok;
    BMarkKind? kind;
    if (at('**')) {
      tok = '**';
      kind = BMarkKind.bold;
    } else if (at('~~')) {
      tok = '~~';
      kind = BMarkKind.strike;
    } else if (at('*') || (at('_') && (i == 0 || !_word(s[i - 1]) || open.containsKey('_')))) {
      tok = s[i];
      kind = BMarkKind.italic;
    }
    if (tok != null && kind != null) {
      final o = open.remove(tok);
      if (o != null) {
        if (b.length > o) marks.add(BMark(start: o, end: b.length, kind: kind, url: ''));
        i += tok.length;
        continue;
      }
      // Only opens when a closing token follows.
      if (s.indexOf(tok, i + tok.length) > i + tok.length) {
        open[tok] = b.length;
        i += tok.length;
        continue;
      }
    }
    b.write(s[i]);
    i++;
  }
  return (b.toString(), normalizeMarks(marks));
}

bool _word(String c) => RegExp(r'[A-Za-z0-9]').hasMatch(c);

// ---------------------------------------------------------------------------
// System clipboard.

/// [tops] as the system clipboard carries them: each kind's clipboard codec
/// (its markdown unless it declares its own).
String clipboardText(List<Node> tops, {BlockRegistry? registry}) {
  final reg = registry ?? blockRegistry;
  return _markdown(tops, (kind) {
    final codecs = reg.codecsOf(kind);
    return codecs.clipboard.lines ?? codecs.markdown;
  });
}

Future<void> copyBlocks(List<Node> tops) => Clipboard.setData(ClipboardData(text: clipboardText(tops)));

Future<String?> readClipboardText() async {
  try {
    return (await Clipboard.getData(Clipboard.kTextPlain))?.text;
  } catch (_) {
    return null;
  }
}
