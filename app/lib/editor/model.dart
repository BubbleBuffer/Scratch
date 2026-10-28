// The editor's document model: a block tree mirrored from vault_core, plus
// its pre-order flattening (what the lazy list shows). Every structural edit
// mutates the tree, re-flattens, and returns the BlockOps to send to
// vault.apply, so the flat list is always the tree's pre-order.

import 'dart:math' as math;

import '../core/models.dart';
import '../registry/block_registry.dart';

class Node {
  Node({
    required this.id,
    required this.kind,
    this.text = '',
    List<BMark>? marks,
    this.checked = false,
    this.target,
    this.color,
    this.width,
    this.numberedStart,
  }) : marks = marks ?? [];

  final String id;

  /// The core's kind name; see the block registry.
  String kind;
  String text;
  List<BMark> marks;
  bool checked;

  /// Page id, attachment id or URL (link, image, file, bookmark blocks).
  String? target;

  /// Block colour (`red`, `red_background`), null for the default.
  String? color;

  /// Image width in percent of the column, null for the full width.
  int? width;

  /// Stored first number of a numbered run; null starts a run at one.
  int? numberedStart;
  Node? parent;
  final List<Node> children = [];

  // Derived by Doc.reflatten().
  int depth = 0;
  int number = 1; // position in a run of numbered siblings
  int flatIndex = 0;

  /// Position in [Doc.visible] (the page's main list), -1 when not there.
  int visIndex = -1;

  bool get isList => blockRegistry.of(kind).listItem;

  BOp setTextOp() => BOp(
      op: OpKind.setText, id: id, kind: kind, text: text, marks: List.of(marks), checked: checked);

  /// This block as the core returns it (for comparing with a reload).
  FlatBlock toFlat() => FlatBlock(
      id: id,
      kind: kind,
      text: text,
      marks: marks,
      checked: checked,
      depth: depth,
      target: target,
      color: color,
      width: width,
      numberedStart: numberedStart);

  /// A copy with a new id (and new ids for the copied children).
  Node copy({bool deep = true}) {
    final c = Node(
        id: newBlockId(),
        kind: kind,
        text: text,
        marks: List.of(marks),
        checked: checked,
        target: target,
        color: color,
        width: width,
        numberedStart: numberedStart);
    if (deep) {
      for (final k in children) {
        final kc = k.copy();
        kc.parent = c;
        c.children.add(kc);
      }
    }
    return c;
  }
}

/// A new block id in the core's format.
String newBlockId() => newId();

BOp _op(OpKind op, String id,
        {String? parent,
        String? after,
        String kind = Kinds.paragraph,
        String text = '',
        List<BMark>? marks,
        bool checked = false,
        String? target,
        String? color,
        int? width,
        int? numberedStart}) =>
    BOp(
        op: op,
        id: id,
        parent: parent,
        after: after,
        kind: kind,
        text: text,
        marks: marks ?? const [],
        checked: checked,
        target: target,
        color: color,
        width: width,
        numberedStart: numberedStart);

class Doc {
  Doc(this.pageId, this.title);
  final String pageId;
  String title;
  final List<Node> roots = [];
  List<Node> flat = [];
  final Map<String, Node> byId = {};

  /// The main list: [flat] without the children of closed toggles and of
  /// blocks that lay out their children themselves (column lists).
  List<Node> visible = [];

  /// Reading order: [flat] without the children of closed toggles.
  List<Node> reading = [];

  /// Whether a toggle shows its children (set by the editor).
  bool Function(Node n) isOpen = (_) => false;

  factory Doc.fromPage(PageData p) {
    final d = Doc(p.id, p.title);
    final stack = <Node>[];
    for (final b in p.blocks) {
      final n = Node(
          id: b.id,
          kind: b.kind,
          text: b.text,
          marks: List.of(b.marks),
          checked: b.checked,
          target: b.target,
          color: b.color,
          width: b.width,
          numberedStart: b.numberedStart);
      while (stack.length > b.depth) {
        stack.removeLast();
      }
      if (stack.isEmpty) {
        d.roots.add(n);
      } else {
        n.parent = stack.last;
        stack.last.children.add(n);
      }
      stack.add(n);
      d.byId[n.id] = n;
    }
    d.reflatten();
    return d;
  }

  /// The blocks as the core would return them.
  List<FlatBlock> toFlat() => [for (final n in flat) n.toFlat()];

  void reflatten() {
    final out = <Node>[];
    void walk(List<Node> list, int depth) {
      int? num;
      for (final n in list) {
        n.depth = depth;
        num = n.kind == Kinds.numbered
            ? (num == null ? n.numberedStart ?? 1 : num + 1)
            : null;
        n.number = num ?? 1;
        n.flatIndex = out.length;
        out.add(n);
        walk(n.children, depth + 1);
      }
    }

    walk(roots, 0);
    flat = out;
    final vis = <Node>[], read = <Node>[];
    void walk2(List<Node> list, bool inVisible) {
      for (final n in list) {
        n.visIndex = inVisible ? vis.length : -1;
        if (inVisible) vis.add(n);
        read.add(n);
        final mode = blockRegistry.of(n.kind).children;
        if (mode == ChildMode.toggle && !isOpen(n)) continue;
        walk2(n.children, inVisible && mode != ChildMode.embedded);
      }
    }

    walk2(roots, true);
    visible = vis;
    reading = read;
  }

  /// The column list (or other block laying out its children) [n] is in.
  Node? embedder(Node n) {
    Node? found;
    for (var p = n.parent; p != null; p = p.parent) {
      if (blockRegistry.of(p.kind).children == ChildMode.embedded) found = p;
    }
    return found;
  }

  /// The rows a block that lays out its children (a column) shows: its
  /// descendants in order, without the children of closed toggles and of
  /// nested column lists (those draw their own).
  List<Node> rowsUnder(Node n) {
    final out = <Node>[];
    void walk(List<Node> list) {
      for (final c in list) {
        out.add(c);
        final mode = blockRegistry.of(c.kind).children;
        if (mode == ChildMode.embedded || (mode == ChildMode.toggle && !isOpen(c))) continue;
        walk(c.children);
      }
    }

    walk(n.children);
    return out;
  }

  /// The closed toggles [n] is inside of.
  List<Node> closedAncestors(Node n) => [
        for (var p = n.parent; p != null; p = p.parent)
          if (blockRegistry.of(p.kind).children == ChildMode.toggle && !isOpen(p)) p
      ];

  List<Node> siblingsOf(Node n) => n.parent?.children ?? roots;

  Node? prevSibling(Node n) {
    final s = siblingsOf(n);
    final i = s.indexOf(n);
    return i > 0 ? s[i - 1] : null;
  }

  Node? nextSibling(Node n) {
    final s = siblingsOf(n);
    final i = s.indexOf(n);
    return i >= 0 && i + 1 < s.length ? s[i + 1] : null;
  }

  Node lastDescendant(Node n) => n.children.isEmpty ? n : lastDescendant(n.children.last);

  bool isInside(Node n, Node ancestor) {
    for (Node? p = n; p != null; p = p.parent) {
      if (p == ancestor) return true;
    }
    return false;
  }

  void _detach(Node n) {
    siblingsOf(n).remove(n);
    n.parent = null;
  }

  void _attach(Node n, Node? parent, Node? after) {
    final list = parent?.children ?? roots;
    final i = after == null ? 0 : list.indexOf(after) + 1;
    list.insert(i, n);
    n.parent = parent;
  }

  // ---- structural edits (mutate, reflatten, return ops) ----

  List<BOp> insertAfter(Node ref, Node n) {
    _attach(n, ref.parent, ref);
    _index(n);
    reflatten();
    return _insertOps(n, ref.parent, ref);
  }

  List<BOp> insertFirstChild(Node? parent, Node n) {
    _attach(n, parent, null);
    _index(n);
    reflatten();
    return _insertOps(n, parent, null);
  }

  /// Inserts [n] as the last child of [parent] (null: the page's end).
  List<BOp> insertLastChild(Node? parent, Node n) {
    final list = parent?.children ?? roots;
    return list.isEmpty ? insertFirstChild(parent, n) : insertAfter(list.last, n);
  }

  void _index(Node n) {
    byId[n.id] = n;
    for (final c in n.children) {
      c.parent = n;
      _index(c);
    }
  }

  /// Insert ops for [n] and its children (the core's insert carries no
  /// target or colour, so those follow as their own ops).
  List<BOp> _insertOps(Node n, Node? parent, Node? after) {
    final ops = [
      _op(OpKind.insert, n.id,
          parent: parent?.id, after: after?.id, kind: n.kind, text: n.text, marks: List.of(n.marks), checked: n.checked,
          numberedStart: n.numberedStart),
      if (n.target != null) _op(OpKind.setTarget, n.id, target: n.target),
      if (n.color != null) _op(OpKind.setColor, n.id, color: n.color),
      if (n.width != null) _op(OpKind.setWidth, n.id, width: n.width),
    ];
    Node? prev;
    for (final c in n.children) {
      ops.addAll(_insertOps(c, n, prev));
      prev = c;
    }
    return ops;
  }

  /// Inserts [n] visually right after [ref]: as its first child when [ref]
  /// has children (so the pre-order stays in place), else as next sibling.
  List<BOp> insertBelow(Node ref, Node n) =>
      ref.children.isNotEmpty ? insertFirstChild(ref, n) : insertAfter(ref, n);

  List<BOp> delete(Node n) {
    void forget(Node x) {
      byId.remove(x.id);
      x.children.forEach(forget);
    }

    _detach(n);
    forget(n);
    reflatten();
    return [_op(OpKind.delete, n.id)];
  }

  List<BOp> move(Node n, Node? parent, Node? after) {
    _detach(n);
    _attach(n, parent, after);
    reflatten();
    return [_op(OpKind.move, n.id, parent: parent?.id, after: after?.id)];
  }

  List<BOp> setKind(Node n, String kind) {
    n.kind = kind;
    reflatten(); // numbering may change
    return [_op(OpKind.setKind, n.id, kind: kind)];
  }

  List<BOp> setChecked(Node n, bool v) {
    n.checked = v;
    return [_op(OpKind.setChecked, n.id, checked: v)];
  }

  List<BOp> setTarget(Node n, String? target) {
    n.target = target;
    return [_op(OpKind.setTarget, n.id, target: target)];
  }

  List<BOp> setColor(Node n, String? color) {
    n.color = color == 'default' ? null : color;
    return [_op(OpKind.setColor, n.id, color: n.color)];
  }

  List<BOp> setWidth(Node n, int? width) {
    n.width = width == null || width >= 100 || width <= 0 ? null : width;
    return [_op(OpKind.setWidth, n.id, width: n.width)];
  }

  /// Applies ops (undo, redo) to this model; the caller sends them on.
  void applyOps(List<BOp> ops) {
    for (final o in ops) {
      final n = byId[o.id];
      switch (o.op) {
        case OpKind.insert:
          final x = Node(id: o.id, kind: o.kind, text: o.text, marks: List.of(o.marks), checked: o.checked,
              numberedStart: o.numberedStart);
          final p = o.parent == null ? null : byId[o.parent];
          final a = o.after == null ? null : byId[o.after];
          _attach(x, p, a);
          byId[x.id] = x;
        case OpKind.delete:
          if (n == null) break;
          void forget(Node x) {
            byId.remove(x.id);
            x.children.forEach(forget);
          }
          _detach(n);
          forget(n);
        case OpKind.move:
          if (n == null) break;
          _detach(n);
          _attach(n, o.parent == null ? null : byId[o.parent], o.after == null ? null : byId[o.after]);
        case OpKind.setText:
          if (n == null) break;
          n.text = o.text;
          n.marks = List.of(o.marks);
        case OpKind.setKind:
          n?.kind = o.kind;
        case OpKind.setChecked:
          n?.checked = o.checked;
        case OpKind.setTarget:
          n?.target = o.target;
        case OpKind.setColor:
          n?.color = o.color;
        case OpKind.setWidth:
          n?.width = o.width;
        case OpKind.setNumberedStart:
          n?.numberedStart = o.numberedStart;
      }
    }
    reflatten();
  }

  /// The blocks' state, for undo (see [diffSnapshots]).
  List<SnapBlock> snapshot() => [
        for (final n in flat)
          SnapBlock(n.id, n.parent?.id, n.kind, n.text, List.of(n.marks), n.checked, n.target, n.color, n.width,
              n.numberedStart)
      ];

  /// Tab: become the last child of the previous sibling.
  List<BOp>? indent(Node n) {
    final prev = prevSibling(n);
    if (prev == null || !blockRegistry.of(prev.kind).acceptsChildren) return null;
    return move(n, prev, prev.children.isEmpty ? null : prev.children.last);
  }

  /// Shift+Tab: become the next sibling of the parent; later siblings become
  /// this block's children so nothing changes place on screen.
  List<BOp>? outdent(Node n) {
    final p = n.parent;
    if (p == null) return null;
    final later = p.children.sublist(p.children.indexOf(n) + 1);
    final ops = <BOp>[];
    for (final s in later) {
      ops.addAll(move(s, n, n.children.isEmpty ? null : n.children.last));
    }
    ops.addAll(move(n, p.parent, p));
    return ops;
  }

  List<BOp>? moveUp(Node n) {
    final prev = prevSibling(n);
    if (prev == null) return null;
    final prevPrev = prevSibling(prev);
    return move(n, n.parent, prevPrev);
  }

  List<BOp>? moveDown(Node n) {
    final next = nextSibling(n);
    if (next == null) return null;
    return move(n, n.parent, next);
  }

  /// Moves [n] (with its children) into the gap above index [gap] of the
  /// main list (0..visible.length). Returns null when the drop would not
  /// change anything or would move a block into itself.
  List<BOp>? dropAt(Node n, int gap) {
    final above = gap > 0 ? visible[gap - 1] : null;
    final below = gap < visible.length ? visible[gap] : null;
    if (above != null && isInside(above, n)) return null;
    if (below == n) return null;
    if (above == null) return move(n, null, null);
    if (below != null && below.depth > above.depth && below != n) {
      return move(n, above, null);
    }
    if (blockRegistry.of(above.kind).children == ChildMode.toggle && isOpen(above) && above.children.isEmpty) {
      return move(n, above, null);
    }
    // Sibling after `above` at its level, or after the ancestor of `above`
    // that sits at the depth of the block below (closing nested lists).
    var anchor = above;
    final targetDepth = below == null ? 0 : math.min(below.depth, above.depth);
    while (anchor.depth > targetDepth && anchor.parent != null) {
      anchor = anchor.parent!;
    }
    if (anchor == n) return null;
    return move(n, anchor.parent, anchor);
  }

  /// Backspace at the start of [n]: append its text to the previous block
  /// in reading order, lift its children into its place, delete it.
  /// Returns (ops, target, caret offset in target) or null.
  (List<BOp>, Node, int)? mergeIntoPrevious(Node n) {
    final i = reading.indexOf(n);
    if (i <= 0) return null;
    final prev = reading[i - 1];
    final ops = <BOp>[];
    if (!blockRegistry.of(prev.kind).editable) {
      // A divider goes away; blocks with content (images, columns) and
      // blocks the app cannot show are kept.
      if (prev.kind != Kinds.divider || prev.children.isNotEmpty) return null;
      ops.addAll(delete(prev));
      return (ops, n, 0);
    }
    final caret = prev.text.length;
    final shift = caret;
    prev.text = prev.text + n.text;
    prev.marks = normalizeMarks([
      ...prev.marks,
      for (final m in n.marks) BMark(start: m.start + shift, end: m.end + shift, kind: m.kind, url: m.url),
    ]);
    ops.add(prev.setTextOp());
    Node after = n;
    for (final c in List.of(n.children)) {
      ops.addAll(move(c, n.parent, after));
      after = c;
    }
    ops.addAll(delete(n));
    return (ops, prev, caret);
  }
}

// ---- undo snapshots ----

class SnapBlock {
  const SnapBlock(
      this.id, this.parent, this.kind, this.text, this.marks, this.checked, this.target, this.color, this.width,
      [this.numberedStart]);
  final String id;
  final String? parent;
  final String kind, text;
  final List<BMark> marks;
  final bool checked;
  final String? target, color;
  final int? width;
  final int? numberedStart;
}

/// The ops that turn the blocks of snapshot [a] into those of [b]: moves
/// and inserts in [b]'s order, content changes, then deletes.
List<BOp> diffSnapshots(List<SnapBlock> a, List<SnapBlock> b) {
  final aById = {for (final x in a) x.id: x};
  final bIds = {for (final x in b) x.id};
  final kids = <String?, List<String>>{};
  final parentOf = <String, String?>{};
  for (final x in a) {
    (kids[x.parent] ??= []).add(x.id);
    parentOf[x.id] = x.parent;
  }
  final lastChild = <String?, String>{};
  final prevInB = <String, String?>{};
  for (final x in b) {
    prevInB[x.id] = lastChild[x.parent];
    lastChild[x.parent] = x.id;
  }
  final ops = <BOp>[];
  for (final x in b) {
    final want = x.parent, after = prevInB[x.id];
    if (parentOf.containsKey(x.id)) {
      final list = kids[parentOf[x.id]]!;
      final i = list.indexOf(x.id);
      if (parentOf[x.id] == want && (i > 0 ? list[i - 1] : null) == after) continue;
      list.removeAt(i);
      ops.add(_op(OpKind.move, x.id, parent: want, after: after));
    } else {
      ops.add(_op(OpKind.insert, x.id,
          parent: want, after: after, kind: x.kind, text: x.text, marks: List.of(x.marks), checked: x.checked,
          numberedStart: x.numberedStart));
      if (x.target != null) ops.add(_op(OpKind.setTarget, x.id, target: x.target));
      if (x.color != null) ops.add(_op(OpKind.setColor, x.id, color: x.color));
      if (x.width != null) ops.add(_op(OpKind.setWidth, x.id, width: x.width));
    }
    final l = kids[want] ??= [];
    l.insert(after == null ? 0 : l.indexOf(after) + 1, x.id);
    parentOf[x.id] = want;
  }
  for (final x in b) {
    final o = aById[x.id];
    if (o == null) continue;
    if (o.kind != x.kind) ops.add(_op(OpKind.setKind, x.id, kind: x.kind));
    if (o.text != x.text || !_sameMarkList(o.marks, x.marks)) {
      ops.add(_op(OpKind.setText, x.id, kind: x.kind, text: x.text, marks: List.of(x.marks), checked: x.checked));
    }
    if (o.checked != x.checked) ops.add(_op(OpKind.setChecked, x.id, checked: x.checked));
    if (o.target != x.target) ops.add(_op(OpKind.setTarget, x.id, target: x.target));
    if (o.color != x.color) ops.add(_op(OpKind.setColor, x.id, color: x.color));
    if (o.width != x.width) ops.add(_op(OpKind.setWidth, x.id, width: x.width));
    if (o.numberedStart != x.numberedStart) {
      ops.add(_op(OpKind.setNumberedStart, x.id, numberedStart: x.numberedStart));
    }
  }
  for (final x in a) {
    if (bIds.contains(x.id)) continue;
    final p = parentOf[x.id];
    if (p == null || bIds.contains(p)) ops.add(_op(OpKind.delete, x.id));
  }
  return ops;
}

bool _sameMarkList(List<BMark> a, List<BMark> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

// ---- marks (UTF-16 offsets on this side) ----

bool _expands(BMarkKind k) =>
    k == BMarkKind.bold ||
    k == BMarkKind.italic ||
    k == BMarkKind.strike ||
    k == BMarkKind.underline ||
    k == BMarkKind.color;

bool sameMark(BMark a, BMark b) => a.kind == b.kind && a.url == b.url;

/// Adjusts marks after the text changed from [oldText] to [newText].
List<BMark> shiftMarks(List<BMark> marks, String oldText, String newText) {
  if (marks.isEmpty) return marks;
  final ol = oldText.length, nl = newText.length;
  var p = 0;
  final maxP = math.min(ol, nl);
  while (p < maxP && oldText.codeUnitAt(p) == newText.codeUnitAt(p)) {
    p++;
  }
  var s = 0;
  while (s < maxP - p && oldText.codeUnitAt(ol - 1 - s) == newText.codeUnitAt(nl - 1 - s)) {
    s++;
  }
  final delStart = p, delEnd = ol - s, ins = nl - s - p;
  final delta = ins - (delEnd - delStart);
  int mapStart(int x) {
    if (x < delStart) return x;
    if (x > delEnd) return x + delta;
    if (x == delEnd) return delStart + ins; // pure insertion before the mark shifts it
    return delStart;
  }

  int mapEnd(int x, bool expand) {
    if (x < delStart) return x;
    if (x > delEnd) return x + delta;
    if (x == delEnd) {
      if (delStart == delEnd) return expand ? x + ins : x; // insertion at the end
      return delStart + ins;
    }
    return delStart;
  }

  final out = <BMark>[];
  for (final m in marks) {
    final a = mapStart(m.start), b = mapEnd(m.end, _expands(m.kind));
    if (a < b) out.add(BMark(start: a, end: b, kind: m.kind, url: m.url));
  }
  return out;
}

/// Sorted, with touching or overlapping marks of the same kind merged.
List<BMark> normalizeMarks(List<BMark> marks) {
  final byKind = <String, List<BMark>>{};
  for (final m in marks) {
    if (m.start >= m.end) continue;
    (byKind['${m.kind.name}|${m.url}'] ??= []).add(m);
  }
  final out = <BMark>[];
  for (final list in byKind.values) {
    list.sort((a, b) => a.start.compareTo(b.start));
    var cur = list.first;
    for (final m in list.skip(1)) {
      if (m.start <= cur.end) {
        cur = BMark(start: cur.start, end: math.max(cur.end, m.end), kind: cur.kind, url: cur.url);
      } else {
        out.add(cur);
        cur = m;
      }
    }
    out.add(cur);
  }
  out.sort((a, b) => a.start != b.start ? a.start.compareTo(b.start) : a.kind.index.compareTo(b.kind.index));
  return out;
}

/// True if [start, end) is fully covered by marks of [kind].
bool rangeHas(List<BMark> marks, BMarkKind kind, int start, int end) {
  if (start >= end) return false;
  final ms = marks.where((m) => m.kind == kind).toList()..sort((a, b) => a.start.compareTo(b.start));
  var pos = start;
  for (final m in ms) {
    if (m.start > pos) break;
    if (m.end > pos) pos = m.end;
    if (pos >= end) return true;
  }
  return pos >= end;
}

/// Sets the text or background colour (by [name]'s family) over
/// [start, end); `default` / `default_background` clears it.
List<BMark> setColorMark(List<BMark> marks, int start, int end, String name) {
  final bg = name.endsWith('_background');
  final out = <BMark>[];
  for (final m in marks) {
    if (m.kind != BMarkKind.color || m.url.endsWith('_background') != bg || m.end <= start || m.start >= end) {
      out.add(m);
      continue;
    }
    if (m.start < start) out.add(BMark(start: m.start, end: start, kind: m.kind, url: m.url));
    if (m.end > end) out.add(BMark(start: end, end: m.end, kind: m.kind, url: m.url));
  }
  if (!name.startsWith('default')) out.add(BMark(start: start, end: end, kind: BMarkKind.color, url: name));
  return normalizeMarks(out);
}

/// Adds or removes [kind] over [start, end).
List<BMark> setMark(List<BMark> marks, BMarkKind kind, int start, int end, bool on, {String url = ''}) {
  final out = <BMark>[];
  for (final m in marks) {
    if (m.kind != kind || m.end <= start || m.start >= end) {
      out.add(m);
      continue;
    }
    // Cut the overlapping part out; it is re-added below if `on`.
    if (m.start < start) out.add(BMark(start: m.start, end: start, kind: m.kind, url: m.url));
    if (m.end > end) out.add(BMark(start: end, end: m.end, kind: m.kind, url: m.url));
  }
  if (on) out.add(BMark(start: start, end: end, kind: kind, url: url));
  return normalizeMarks(out);
}

List<BMark> sliceMarks(List<BMark> marks, int start, int end) {
  final out = <BMark>[];
  for (final m in marks) {
    final a = math.max(m.start, start), b = math.min(m.end, end);
    if (a < b) out.add(BMark(start: a - start, end: b - start, kind: m.kind, url: m.url));
  }
  return out;
}

