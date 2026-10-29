// The `view` block kind: a page's child pages in a layout (list, table,
// board, calendar, gallery, timeline). The block text is the view's
// settings JSON; the target is the page whose children it shows (none: the
// page holding the block). Consecutive view blocks are one group shown as
// tabs: the first block of a run draws the tabs and the active view, the
// others draw nothing of their own. Writes go back into the blocks' text
// through the editor, so they are page edits with undo.

import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../core/vault.dart';
import '../../../registry/block_codecs.dart';
import '../../../registry/block_registry.dart';
import '../../../editor/model.dart';
import '../../../state/app_state.dart';
import '../../../state/collection_store.dart';
import '../../../ui/database/values.dart';
import '../../../ui/icon_x.dart';
import '../database_view.dart';

/// The core's kind name of a view block.
const viewKind = 'view';

/// The blocks of [n]'s run of view blocks, [n] first; empty when [n]
/// follows another view block (that block draws the group).
List<Node> viewRun(BlockHost host, Node n) {
  final siblings = host is BlockEditingHost
      ? host.siblingsOf(n)
      : (n.parent?.children ?? [for (final x in host.readingOrder) if (x.parent == null) x]);
  final i = siblings.indexOf(n);
  if (i < 0) return [n];
  if (i > 0 && siblings[i - 1].kind == viewKind) return const [];
  return [
    for (var j = i; j < siblings.length && siblings[j].kind == viewKind; j++) siblings[j],
  ];
}

Widget _body(BlockContext b) {
  final run = viewRun(b.host, b.node);
  if (run.isEmpty) return const SizedBox.shrink();
  return LayoutBuilder(
    builder: (context, box) => ViewBlockGroup(
      key: ValueKey('view-group-${b.node.id}'),
      nodes: run,
      host: b.host,
      width: box.maxWidth,
    ),
  );
}

double _handleTop(BlockContext b) => 6;

/// The view fence of the core's markdown: the settings JSON, with `page`
/// for a source other than the page holding the block.
List<String> _viewFence(Node n) {
  Object? data;
  try {
    data = n.text.trim().isEmpty ? <String, Object?>{} : jsonDecode(n.text);
  } catch (_) {
    data = null;
  }
  final map = data is Map ? Map<String, Object?>.from(data) : <String, Object?>{};
  map['source'] ??= 'contents';
  final target = n.target;
  if (target != null && target.isNotEmpty) map['page'] = target;
  return ['```view', jsonEncode(map), '```'];
}

final viewBlockCodecs = BlockCodecs(
  markdown: MarkdownCodec((n, _) => _viewFence(n)),
  clipboard: const ClipboardCodec.viaMarkdown(),
  // Settings are not text: a view contributes nothing to plain text.
  plainText: PlainTextCodec((n) => ''),
);

/// Inserted from `/`: an empty text is a List view (the core's default);
/// no target is the page holding the block.
final viewBlock = BlockType(
  kind: viewKind,
  label: 'View',
  hint: 'List the pages inside this page.',
  iconX: Ix.table,
  aliases: 'view views list table board calendar gallery timeline pages children subpages',
  editable: false,
  acceptsChildren: false,
  colorable: false,
  insertOnly: true,
  body: _body,
  handleTop: _handleTop,
  codecs: viewBlockCodecs,
);

/// One run of view blocks: the tabs and the active view.
class ViewBlockGroup extends StatefulWidget {
  const ViewBlockGroup({super.key, required this.nodes, required this.host, required this.width});
  final List<Node> nodes;
  final BlockHost host;
  final double width;

  @override
  State<ViewBlockGroup> createState() => _ViewBlockGroupState();
}

class _ViewBlockGroupState extends State<ViewBlockGroup> {
  late final _BlockViews _source = _BlockViews(this);

  /// Parsed settings by block id, with the text they were parsed from.
  final Map<String, (String, ViewDef)> _parsed = {};
  bool _parsing = false;
  List<ViewDef> _views = const [];
  bool _ready = false;
  String? _active;

  Vault get _vault => context.app.vault;
  String get _firstId => widget.nodes.first.id;

  @override
  void initState() {
    super.initState();
    _active = viewBlockTabs[widget.nodes.first.id];
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _sync();
  }

  @override
  void didUpdateWidget(ViewBlockGroup old) {
    super.didUpdateWidget(old);
    _sync();
  }

  @override
  void dispose() {
    _source.dispose();
    super.dispose();
  }

  Node? _node(String id) {
    for (final n in widget.nodes) {
      if (n.id == id) return n;
    }
    return null;
  }

  /// Takes the blocks' current settings, parsing texts not seen yet.
  void _sync() {
    final missing = [
      for (final n in widget.nodes)
        if (_parsed[n.id]?.$1 != n.text) n,
    ];
    if (missing.isEmpty) {
      _publish();
      return;
    }
    if (_parsing) return;
    _parsing = true;
    final vault = _vault;
    Future.wait([
      for (final n in missing)
        vault.run((v) => v.viewBlockDef(id: n.id, text: n.text)).then((d) => (n.id, n.text, d)),
    ]).then((results) {
      _parsing = false;
      if (!mounted) return;
      for (final (id, text, d) in results) {
        _parsed[id] = (text, d);
      }
      setState(_sync);
    }, onError: (Object e) {
      _parsing = false;
      debugPrint('view block settings: $e');
    });
  }

  void _publish() {
    final next = [for (final n in widget.nodes) ?_parsed[n.id]?.$2];
    final changed = !listEquals(next, _views);
    _views = next;
    final first = !_ready;
    _ready = true;
    if (first || !changed) return;
    // The store rebuilds its listeners: not while this frame is built.
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (mounted) _source.changed();
    });
  }

  /// The page whose children the group's active tab shows.
  String _sourceOf(String? viewId) {
    final n = (viewId == null ? null : _node(viewId)) ?? widget.nodes.first;
    final t = n.target;
    return t == null || t.isEmpty ? widget.host.pageId : t;
  }

  void _onActive(String id) {
    final before = _sourceOf(_active);
    _active = id;
    if (_sourceOf(id) != before && mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    if (!_ready) return const SizedBox(height: 120);
    final source = _sourceOf(_active);
    return DatabaseView(
      key: ValueKey('view-blocks-$_firstId-$source'),
      collection: source,
      viewSource: _source,
      tabsKey: _firstId,
      onActiveView: _onActive,
      width: widget.width,
    );
  }

  // ------------------------------------------------------------ writes

  Future<void> save(ViewDef v) async {
    final n = _node(v.id);
    if (n == null) return;
    final text = await _vault.run((x) => x.viewBlockText(view: v));
    if (!mounted || _node(v.id) != n) return;
    _parsed[n.id] = (text, v);
    _views = [for (final x in _views) x.id == v.id ? v : x];
    widget.host.setText(n, text);
  }

  Future<String> add(ViewDef v, {String? after}) async {
    final host = widget.host;
    final anchor = (after == null ? null : _node(after)) ?? widget.nodes.last;
    final text = await _vault.run((x) => x.viewBlockText(view: v));
    if (!mounted || host is! BlockEditingHost) return '';
    final id = host.insertBlock(anchor, viewKind, text, target: anchor.target);
    _parsed[id] = (text, v.copyWith(id: id));
    return id;
  }

  Future<void> delete(ViewDef v) async {
    final host = widget.host;
    final n = _node(v.id);
    if (n == null || host is! BlockEditingHost) return;
    _parsed.remove(n.id);
    host.deleteBlock(n);
  }
}

class _BlockViews extends ViewSource {
  _BlockViews(this._group);
  final _ViewBlockGroupState _group;

  @override
  List<ViewDef> get views => _group._views;

  void changed() => notifyListeners();

  @override
  Future<void> save(ViewDef v) => _group.save(v);

  @override
  Future<String> add(ViewDef v, {String? after}) => _group.add(v, after: after);

  @override
  Future<void> delete(ViewDef v) => _group.delete(v);
}

/// A page of the database type without view blocks: a List of its child
/// pages under the body, not saved. Changing it, or adding a view, writes it
/// into the page as a view block first.
class PageChildrenView extends StatefulWidget {
  const PageChildrenView({super.key, required this.pageId, required this.append, required this.width});
  final String pageId;

  /// Appends a block of a kind with a text to the page; returns its id.
  final String Function(String kind, String text) append;
  final double width;

  @override
  State<PageChildrenView> createState() => _PageChildrenViewState();
}

class _PageChildrenViewState extends State<PageChildrenView> {
  late final _ImplicitViews _source = _ImplicitViews(this);

  late final ViewDef _list = ViewDef(
    id: 'children-${widget.pageId}',
    name: 'List',
    kind: ViewKind.list,
    sorts: const [],
    visibleProps: const [],
    config: '',
  );

  @override
  void dispose() {
    _source.dispose();
    super.dispose();
  }

  Future<String> _append(ViewDef v) async {
    final text = await context.app.vault.run((x) => x.viewBlockText(view: v));
    if (!mounted) return '';
    return widget.append(viewKind, text);
  }

  Future<void> save(ViewDef v) async {
    final id = await _append(v);
    if (id.isNotEmpty) viewBlockTabs[id] = id;
  }

  Future<String> add(ViewDef v) async {
    final first = await _append(_list);
    if (first.isEmpty) return '';
    final id = await _append(v);
    viewBlockTabs[first] = id;
    return id;
  }

  @override
  Widget build(BuildContext context) => DatabaseView(
    key: ValueKey('page-children-${widget.pageId}'),
    collection: widget.pageId,
    viewSource: _source,
    width: widget.width,
  );
}

class _ImplicitViews extends ViewSource {
  _ImplicitViews(this._view);
  final _PageChildrenViewState _view;

  @override
  List<ViewDef> get views => [_view._list];

  @override
  Future<void> save(ViewDef v) => _view.save(v);

  @override
  Future<String> add(ViewDef v, {String? after}) => _view.add(v);

  @override
  Future<void> delete(ViewDef v) async {}
}
