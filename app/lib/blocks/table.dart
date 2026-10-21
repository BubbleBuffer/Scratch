// Table: a GFM pipe table. The core keeps the block's text as the table's
// markdown, verbatim, and this block draws it at render time: a header row,
// the body rows, each column's alignment from the delimiter row, and cells
// with bold, italic, code and link marks. Harder inline syntax shows as typed,
// and text that is not a table shows as plain text. The block is read-only (a
// tap on a link opens it; a tap elsewhere selects the block). A table wider
// than the block scrolls sideways.
//
// Links: an external link (http, https, mailto) opens in the browser. A link
// to a page in the vault (a relative .md link, or a [[wiki link]]) opens that
// page through the block host. The core marks the source with a Link (the
// original target) and a PageLink (the page id) on the label; see
// [pageLinkTargets] for how those marks resolve a target to a page.
//
// Cells split on unescaped pipes. A pipe inside a code span does not split a
// cell, and `\|` is a literal pipe everywhere.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:url_launcher/url_launcher.dart';

import '../core/models.dart';
import '../editor/model.dart';
import '../registry/block_codecs.dart';
import '../registry/block_registry.dart';
import '../ui/ui.dart';

/// A parsed table: the header and body rows as inline markdown, and each
/// column's alignment. Every row has one cell per column.
class MarkdownTable {
  const MarkdownTable({
    required this.header,
    required this.rows,
    required this.alignments,
  });

  final List<String> header;
  final List<List<String>> rows;
  final List<TextAlign> alignments;

  int get columns => header.length;
}

/// Columns never get narrower than this; a table wider than the block scrolls.
const _minColumnWidth = 140.0;

final _delimiterCell = RegExp(r'^:?-+:?$');
final _punctuation = RegExp(r'[!-/:-@\[-`{-~]');
final _wordChar = RegExp(r'[\p{L}\p{N}]', unicode: true);

/// Opens a table link. A test replaces it to see what a tap would open.
@visibleForTesting
void Function(Uri uri) openTableLink = _launchLink;

void _launchLink(Uri uri) {
  launchUrl(uri, mode: LaunchMode.externalApplication);
}

/// The Uri a link opens: http or https with a host, or mailto. Anything else
/// (relative file links, other schemes) is null and shows as plain text.
Uri? safeLinkUri(String raw) {
  final uri = Uri.tryParse(raw);
  if (uri == null) return null;
  return switch (uri.scheme) {
    'http' || 'https' => uri.host.isEmpty ? null : uri,
    'mailto' => uri,
    _ => null,
  };
}

/// The page each link target in a table's source resolves to, keyed by the
/// target as written. The core marks a link's label with a Link (its original
/// target) and a PageLink (the page id) over the same range. Loro may merge
/// adjacent PageLinks, so one PageLink can cover several Links: a Link counts
/// when its range lies inside a PageLink's. A target maps to the first page
/// that claims it.
Map<String, String> pageLinkTargets(List<BMark> marks) {
  final out = <String, String>{};
  for (final page in marks) {
    if (page.kind != BMarkKind.pageLink) continue;
    for (final link in marks) {
      if (link.kind != BMarkKind.link || link.url.isEmpty) continue;
      if (link.start >= page.start && link.end <= page.end) {
        out.putIfAbsent(link.url, () => page.url);
      }
    }
  }
  return out;
}

/// The table [text] holds, or null when it is not one: a header row and a
/// delimiter row (both with pipes and the same number of cells), then body
/// rows up to the end of the text. A blank line followed by more text is not
/// a table.
MarkdownTable? parseMarkdownTable(String text) {
  final lines = text
      .replaceAll('\r\n', '\n')
      .split('\n')
      .map((line) => line.trim())
      .toList();
  while (lines.isNotEmpty && lines.first.isEmpty) {
    lines.removeAt(0);
  }
  while (lines.isNotEmpty && lines.last.isEmpty) {
    lines.removeLast();
  }
  if (lines.length < 2 || !lines[0].contains('|') || !lines[1].contains('|')) {
    return null;
  }
  final header = splitTableRow(lines[0]);
  final delimiter = splitTableRow(lines[1]);
  if (delimiter.length != header.length ||
      !delimiter.every(_delimiterCell.hasMatch)) {
    return null;
  }
  final rows = <List<String>>[];
  for (final line in lines.skip(2)) {
    if (line.isEmpty) return null;
    final cells = splitTableRow(line);
    rows.add([
      for (var i = 0; i < header.length; i++) i < cells.length ? cells[i] : '',
    ]);
  }
  return MarkdownTable(
    header: header,
    rows: rows,
    alignments: [for (final d in delimiter) _alignment(d)],
  );
}

TextAlign _alignment(String delimiter) {
  final left = delimiter.startsWith(':');
  final right = delimiter.endsWith(':');
  if (left && right) return TextAlign.center;
  if (right) return TextAlign.end;
  return TextAlign.start;
}

/// The cells of one table line, as inline markdown. Outer pipes are optional.
/// Escaped pipes (`\|`) become literal pipes, and a code span is copied whole,
/// so a pipe inside it does not split the cell.
List<String> splitTableRow(String line) {
  var s = line.trim();
  if (s.startsWith('|')) s = s.substring(1);
  if (s.endsWith('|') && !s.endsWith(r'\|')) s = s.substring(0, s.length - 1);
  final cells = <String>[];
  final buf = StringBuffer();
  var i = 0;
  while (i < s.length) {
    final ch = s[i];
    if (ch == r'\' && i + 1 < s.length && s[i + 1] == '|') {
      buf.write('|');
      i += 2;
    } else if (ch == '`') {
      final run = _runLength(s, i, '`');
      final close = _closingBackticks(s, i + run, run);
      if (close < 0) {
        buf.write('`' * run);
        i += run;
      } else {
        buf.write(s.substring(i, close + run));
        i = close + run;
      }
    } else if (ch == '|') {
      cells.add(buf.toString().trim());
      buf.clear();
      i++;
    } else {
      buf.write(ch);
      i++;
    }
  }
  cells.add(buf.toString().trim());
  return cells;
}

/// Inline marks a cell can show.
enum _Mark { bold, italic, code, link }

/// Where a link goes: a page in the vault, or an external address.
class _Target {
  const _Target.page(String id)
    : pageId = id,
      uri = null;
  const _Target.external(Uri link)
    : pageId = null,
      uri = link;

  final String? pageId;
  final Uri? uri;

  /// Identifies the target across rebuilds: one tap recognizer per key.
  String get key => pageId != null ? 'page:$pageId' : 'url:$uri';
}

/// A run of cell text with its marks, and the target it links to (null when it
/// is not a link this block opens).
class _Run {
  const _Run(this.text, this.marks, [this.target]);

  final String text;
  final Set<_Mark> marks;
  final _Target? target;
}

/// The target of a link written as [href]: the page it resolves to in [pages],
/// else an external Uri [safeLinkUri] accepts, else null (plain text).
_Target? _targetFor(String href, Map<String, String> pages) {
  final page = pages[href];
  if (page != null) return _Target.page(page);
  final uri = safeLinkUri(href);
  return uri == null ? null : _Target.external(uri);
}

/// A cell's text as runs of marked text: `code`, **bold** or __bold__,
/// *italic* or _italic_, [text](href) and [[wiki]] or [[wiki|alias]]. A link
/// that resolves (see [_targetFor]) is marked as a link and carries its
/// target. A `[[wiki]]` with no page stays as typed. Other links show as their
/// plain text. A backslash before punctuation is that character. Anything
/// unmatched is plain text.
List<_Run> _inlineRuns(
  String s, {
  Map<String, String> pages = const {},
  Set<_Mark> marks = const {},
  _Target? target,
}) {
  final out = <_Run>[];
  final buf = StringBuffer();
  void flush() {
    if (buf.length == 0) return;
    out.add(_Run(buf.toString(), marks, target));
    buf.clear();
  }

  var i = 0;
  while (i < s.length) {
    final ch = s[i];
    if (ch == r'\' && i + 1 < s.length && _punctuation.hasMatch(s[i + 1])) {
      buf.write(s[i + 1]);
      i += 2;
      continue;
    }
    if (ch == '`') {
      final run = _runLength(s, i, '`');
      final close = _closingBackticks(s, i + run, run);
      if (close < 0) {
        buf.write('`' * run);
        i += run;
      } else {
        flush();
        out.add(_Run(
          _codeText(s.substring(i + run, close)),
          {...marks, _Mark.code},
          target,
        ));
        i = close + run;
      }
      continue;
    }
    if (ch == '[' && s.startsWith('[[', i)) {
      final wiki = _wikiAt(s, i);
      if (wiki != null) {
        final (inner, end) = wiki;
        final bar = inner.indexOf('|');
        final name = (bar < 0 ? inner : inner.substring(0, bar)).trim();
        final alias = bar < 0 ? '' : inner.substring(bar + 1).trim();
        final page = pages[inner] ?? pages[name];
        if (page == null) {
          buf.write(s.substring(i, end));
        } else {
          flush();
          out.addAll(_inlineRuns(
            alias.isEmpty ? name : alias,
            pages: pages,
            marks: {...marks, _Mark.link},
            target: _Target.page(page),
          ));
        }
        i = end;
        continue;
      }
    }
    if (ch == '[') {
      final link = _linkAt(s, i);
      if (link != null) {
        flush();
        final (text, href, end) = link;
        final to = _targetFor(href, pages);
        out.addAll(_inlineRuns(
          text,
          pages: pages,
          marks: to == null ? marks : {...marks, _Mark.link},
          target: to ?? target,
        ));
        i = end;
        continue;
      }
    }
    if (ch == '*' || ch == '_') {
      final run = _runLength(s, i, ch);
      final close = run <= 2 ? _emphasisClose(s, i, run) : -1;
      if (close < 0) {
        buf.write(ch * run);
        i += run;
      } else {
        flush();
        final mark = run == 2 ? _Mark.bold : _Mark.italic;
        out.addAll(_inlineRuns(
          s.substring(i + run, close),
          pages: pages,
          marks: {...marks, mark},
          target: target,
        ));
        i = close + run;
      }
      continue;
    }
    buf.write(ch);
    i++;
  }
  flush();
  return out;
}

/// A code span's text: `\|` reads as a pipe, and one space is dropped from
/// each side when both are there.
String _codeText(String raw) {
  final t = raw.replaceAll(r'\|', '|');
  if (t.length > 2 && t.startsWith(' ') && t.endsWith(' ') && t.trim().isNotEmpty) {
    return t.substring(1, t.length - 1);
  }
  return t;
}

/// The text, href and index after a `[text](href)` starting at [i], or null
/// when there is no such link (no text, no `(`, no closing `)`, or an empty or
/// spaced href).
(String, String, int)? _linkAt(String s, int i) {
  final close = s.indexOf(']', i + 1);
  if (close <= i + 1 || close + 1 >= s.length || s[close + 1] != '(') return null;
  final end = s.indexOf(')', close + 2);
  if (end < 0) return null;
  final href = s.substring(close + 2, end).trim();
  if (href.isEmpty || href.contains(RegExp(r'\s'))) return null;
  return (s.substring(i + 1, close), href, end + 1);
}

/// The inner text of a `[[...]]` starting at [i] and the index after it, or
/// null when there is no closing `]]` on the same line.
(String, int)? _wikiAt(String s, int i) {
  final close = s.indexOf(']]', i + 2);
  if (close < 0) return null;
  final inner = s.substring(i + 2, close);
  if (inner.trim().isEmpty || inner.contains('\n')) return null;
  return (inner, close + 2);
}

/// The index of the delimiter closing the emphasis opened at [i] (a run of [n]
/// [ch]), or -1. The opener must be followed by a non-space, the closer
/// preceded by one, and an underscore must not sit inside a word.
int _emphasisClose(String s, int i, int n) {
  final ch = s[i];
  final after = i + n;
  if (after >= s.length || _isSpace(s[after])) return -1;
  if (ch == '_' && i > 0 && _wordChar.hasMatch(s[i - 1])) return -1;
  var j = after;
  while (j < s.length) {
    final at = s.indexOf(ch, j);
    if (at < 0) return -1;
    final run = _runLength(s, at, ch);
    final closes = run == n &&
        at > after &&
        !_isSpace(s[at - 1]) &&
        (ch == '*' || at + n >= s.length || !_wordChar.hasMatch(s[at + n]));
    if (closes) return at;
    j = at + run;
  }
  return -1;
}

/// The index of the next run of exactly [n] backticks at or after [from], or
/// -1.
int _closingBackticks(String s, int from, int n) {
  var j = from;
  while (true) {
    final at = s.indexOf('`', j);
    if (at < 0) return -1;
    final run = _runLength(s, at, '`');
    if (run == n) return at;
    j = at + run;
  }
}

int _runLength(String s, int i, String ch) {
  var n = 0;
  while (i + n < s.length && s[i + n] == ch) {
    n++;
  }
  return n;
}

bool _isSpace(String c) => c.trim().isEmpty;

TextSpan _span(
  _Run run,
  TextStyle base,
  AppColors c,
  TypeScale ty,
  TapGestureRecognizer Function(_Target to) linkFor,
) {
  var style = base;
  if (run.marks.contains(_Mark.code)) {
    style = ts(ty.code, c.inlineCodeText, mono: true).copyWith(backgroundColor: c.inlineCodeBg);
  } else {
    if (run.marks.contains(_Mark.bold)) style = style.copyWith(fontWeight: FontWeight.w700);
    if (run.marks.contains(_Mark.italic)) style = style.copyWith(fontStyle: FontStyle.italic);
  }
  if (run.marks.contains(_Mark.link)) {
    style = style.copyWith(color: c.accentText, decoration: TextDecoration.underline);
  }
  final to = run.target;
  return TextSpan(
    text: run.text,
    style: style,
    recognizer: to == null ? null : linkFor(to),
  );
}

Widget _cell(
  String markdown,
  TextAlign align,
  TextStyle base,
  AppColors c,
  TypeScale ty,
  Map<String, String> pages,
  TapGestureRecognizer Function(_Target to) linkFor,
) {
  final runs = _inlineRuns(markdown, pages: pages);
  return Padding(
    padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
    child: Text.rich(
      TextSpan(
        style: base,
        children: runs.isEmpty
            ? [TextSpan(text: ' ', style: base)]
            : [for (final r in runs) _span(r, base, c, ty, linkFor)],
      ),
      textAlign: align,
    ),
  );
}

class _TableView extends StatefulWidget {
  const _TableView({
    super.key,
    required this.id,
    required this.table,
    required this.style,
    required this.pages,
    required this.host,
  });

  final String id;
  final MarkdownTable table;
  final TextStyle style;

  /// Link target (as written) to page id, from [pageLinkTargets].
  final Map<String, String> pages;
  final BlockHost host;

  @override
  State<_TableView> createState() => _TableViewState();
}

class _TableViewState extends State<_TableView> {
  /// One tap recognizer per link target, owned here and disposed with the view.
  final _links = <String, TapGestureRecognizer>{};

  /// The target keys the latest build's cells use. Recognizers for keys that
  /// have dropped out are disposed after the frame.
  Set<String> _liveKeys = {};

  TapGestureRecognizer _linkFor(_Target to) => _links.putIfAbsent(
    to.key,
    () => TapGestureRecognizer()..onTap = () => _open(to),
  );

  void _open(_Target to) {
    final page = to.pageId;
    if (page != null) {
      widget.host.openPage(page);
      return;
    }
    final uri = to.uri;
    if (uri != null) openTableLink(uri);
  }

  /// The link keys every cell of [table] uses now.
  Set<String> _keysOf(MarkdownTable table) {
    final keys = <String>{};
    void collect(String cell) {
      for (final run in _inlineRuns(cell, pages: widget.pages)) {
        final to = run.target;
        if (to != null) keys.add(to.key);
      }
    }

    table.header.forEach(collect);
    for (final row in table.rows) {
      row.forEach(collect);
    }
    return keys;
  }

  void _pruneAfterFrame(Set<String> live) {
    _liveKeys = live;
    if (!_links.keys.any((key) => !live.contains(key))) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _links.removeWhere((key, recognizer) {
        if (_liveKeys.contains(key)) return false;
        recognizer.dispose();
        return true;
      });
    });
  }

  @override
  void dispose() {
    for (final r in _links.values) {
      r.dispose();
    }
    _links.clear();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final table = widget.table;
    final pages = widget.pages;
    final head = widget.style.copyWith(fontWeight: FontWeight.w600);
    final rule = BorderSide(color: c.borderSubtle);
    _pruneAfterFrame(_keysOf(table));
    // The layout runs inside the border, so the width it reads is the width
    // the table gets (the border's 1 px a side is already taken off).
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: c.border),
        borderRadius: BorderRadius.circular(Radii.sm),
      ),
      clipBehavior: Clip.antiAlias,
      child: LayoutBuilder(
        builder: (context, box) {
          final fit = box.maxWidth.isFinite ? box.maxWidth : 0.0;
          final width = math.max(fit, table.columns * _minColumnWidth);
          return SingleChildScrollView(
            key: ValueKey('table-scroll-${widget.id}'),
            scrollDirection: Axis.horizontal,
            child: SizedBox(
              width: width,
              child: Table(
                border: TableBorder(horizontalInside: rule, verticalInside: rule),
                children: [
                  TableRow(
                    decoration: BoxDecoration(color: c.bgSecondary),
                    children: [
                      for (var i = 0; i < table.columns; i++)
                        _cell(table.header[i], table.alignments[i], head, c, ty, pages, _linkFor),
                    ],
                  ),
                  for (final row in table.rows)
                    TableRow(children: [
                      for (var i = 0; i < table.columns; i++)
                        _cell(row[i], table.alignments[i], widget.style, c, ty, pages, _linkFor),
                    ]),
                ],
              ),
            ),
          );
        },
      ),
    );
  }
}

TextStyle _style(Node n, AppColors c, TypeScale ty) => ts(ty.body, c.textPrimary);

Widget _body(BlockContext b) {
  final table = parseMarkdownTable(b.node.text);
  if (table == null) {
    if (b.node.text.trim().isEmpty) return SizedBox(height: b.lineHeight);
    return Text(b.node.text, style: b.style);
  }
  return _TableView(
    key: ValueKey('table-${b.node.id}'),
    id: b.node.id,
    table: table,
    style: b.style,
    pages: pageLinkTargets(b.node.marks),
    host: b.host,
  );
}

List<String> _tableLines(Node n, String inline) => n.text.split('\n');

String _tableText(Node n) => n.text;

// The table's markdown, verbatim: the text is the GFM source.
const _tableCodecs = BlockCodecs(
  markdown: MarkdownCodec(_tableLines),
  clipboard: ClipboardCodec.viaMarkdown(),
  plainText: PlainTextCodec(_tableText),
);

const tableBlock = BlockType(
  kind: Kinds.table,
  label: 'Table',
  hint: 'A table, written as markdown.',
  aliases: 'table grid',
  editable: false,
  inMenu: false,
  acceptsChildren: false,
  colorable: false,
  textStyle: _style,
  body: _body,
  codecs: _tableCodecs,
);
