// The search card: one `query` block drawn as its live results. It owns one
// [QueryWatch], which holds the core watch. The watch is released on dispose
// and whenever the block's query text changes, so no registration outlives
// its card. The watch's state lives in query_watch.dart.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/vault.dart';
import '../../registry/block_registry.dart';
import '../../services/legacy_host.dart' show openPage;
import '../../src/rust/api/types.dart';
import '../../ui/ui.dart';
import 'query_row.dart';
import 'query_toast.dart';
import 'query_watch.dart';

export 'query_watch.dart' show QueryBridge, queryBridge, queryWatchLimit;

/// The header's overflow offers "Show inline", which turns the block into a
/// paragraph holding its chip (see `_showInline`).
const showInlineMenuItem = true;

class QueryCard extends StatefulWidget {
  const QueryCard({
    super.key,
    required this.block,
    required this.vault,
    required this.tasksOn,
  });

  /// The block being drawn: its query text, its editor host and whether
  /// it is read-only (document previews show results but cannot edit).
  final BlockContext block;
  final Vault vault;

  /// Whether the Tasks add-on is on. Off means task rows have no checkbox.
  final bool tasksOn;

  @override
  State<QueryCard> createState() => _QueryCardState();
}

class _QueryCardState extends State<QueryCard> {
  late final QueryWatch _watch = QueryWatch(
    vault: widget.vault,
    tasksOn: widget.tasksOn,
    query: widget.block.node.text,
  );
  late final StreamSubscription<Hit> _leftSub;
  VoidCallback? _dismissToast;

  bool _expanded = false;
  bool _collapsed = false;
  bool _editing = false;
  final _edit = TextEditingController();
  final _editFocus = FocusNode();

  @override
  void initState() {
    super.initState();
    _leftSub = _watch.rowsLeft.listen(_showLeftToast);
    _watch.onWriteFailed = _onWriteFailed;
    _watch.addListener(_onWatch);
    _watch.start();
    if (!widget.block.readOnly && _watch.query.trim().isEmpty) {
      _editing = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _editing) _editFocus.requestFocus();
      });
    }
  }

  @override
  void didUpdateWidget(QueryCard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.block.node.text != _watch.query) {
      _expanded = false;
      _watch.setQuery(widget.block.node.text);
    }
  }

  @override
  void dispose() {
    _dismissToast?.call();
    _leftSub.cancel();
    _watch.dispose();
    _edit.dispose();
    _editFocus.dispose();
    super.dispose();
  }

  void _onWatch() {
    if (mounted) setState(() {});
  }

  void _onWriteFailed(Object error) {
    _toastError();
  }

  // ---- editing the query ----

  void _startEdit() {
    if (widget.block.readOnly) return;
    _edit.value = TextEditingValue(
      text: _watch.query,
      selection: TextSelection.collapsed(offset: _watch.query.length),
    );
    setState(() => _editing = true);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && _editing) _editFocus.requestFocus();
    });
  }

  void _applyEdit() {
    final text = _edit.text.trim();
    setState(() => _editing = false);
    if (text != widget.block.node.text) widget.block.host.setText(widget.block.node, text);
  }

  void _cancelEdit() {
    setState(() => _editing = false);
  }

  KeyEventResult _onEditKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      _cancelEdit();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  // ---- ticking tasks: the toast ----

  void _showLeftToast(Hit hit) {
    _dismissToast?.call();
    _dismissToast = showQueryToast(
      context,
      'Marked done, so it left this list.',
      undo: () => _watch.setDone(hit, done: false),
      onError: (_) => _toastError(),
    );
  }

  void _toastError() {
    if (!mounted) return;
    _dismissToast?.call();
    _dismissToast = showQueryToast(context, "Couldn't update that task.");
  }

  /// "Show inline": the block becomes a paragraph holding just its chip, over
  /// the current label. It is one undo step. Needs the first result, so the
  /// label is known.
  void _showInline() {
    if (!_watch.ready) return;
    final node = widget.block.node;
    final label = _watch.wording.chipLabel;
    widget.block.host.replaceWithParagraph(node, label, [
      BMark(start: 0, end: label.length, kind: BMarkKind.query, url: node.text),
    ]);
  }

  void _openOverflow(BuildContext context) {
    showPopover<void>(
      context,
      globalRectOf(context),
      (menuContext) => MenuList(
        children: [
          MenuItem(
            label: 'Show inline',
            onTap: () {
              Navigator.of(menuContext).pop();
              _showInline();
            },
          ),
        ],
      ),
    );
  }

  // ---- drawing ----

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final hits = _watch.hits;
    final visible = _watch.visibleHits;
    // Rows hidden by a pending tick are still counted by the core until its
    // answer arrives; the count leaves them out.
    final count = _watch.count;
    final shown = _expanded ? visible : visible.take(queryPreviewRows).toList();
    final more = visible.length - shown.length;
    final wording = _watch.wording;
    return Container(
      key: ValueKey('query-card-${widget.block.node.id}'),
      decoration: BoxDecoration(
        color: c.bgPrimary,
        border: Border.all(color: c.border),
        borderRadius: Radii.panel,
        boxShadow: c.seam,
      ),
      clipBehavior: Clip.antiAlias,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _header(context, count, hits.isNotEmpty),
          if (_editing) _editHint(context),
          if (!_collapsed) ...[
            if (_watch.query.trim().isEmpty)
              const QueryEmptyLine(text: 'Type a query to see results.')
            else if (_watch.error != null)
              const QueryEmptyLine(text: "Couldn't run this query.")
            else if (_watch.ready && visible.isEmpty)
              QueryEmptyLine(text: wording.emptyLine)
            else
              for (var i = 0; i < shown.length; i++)
                DecoratedBox(
                  decoration: BoxDecoration(
                    border: i == 0
                        ? null
                        : Border(top: BorderSide(color: c.borderSubtle)),
                  ),
                  child: _row(context, shown[i]),
                ),
            if (more > 0)
              Align(
                alignment: Alignment.centerLeft,
                child: Padding(
                  padding: const EdgeInsets.fromLTRB(8, 0, 8, 6),
                  child: TextBtn(
                    'Show $more more',
                    key: ValueKey('query-more-${widget.block.node.id}'),
                    onTap: () => setState(() => _expanded = true),
                  ),
                ),
              ),
          ],
        ],
      ),
    );
  }

  Widget _row(BuildContext context, Hit hit) {
    final info = _watch.infoOf(hit);
    final checkable = _watch.isTaskRow(hit);
    return QueryRowTile(
      hit: hit,
      done: _watch.isDone(hit),
      pageIcon: info?.icon,
      statusLabel: info?.statusLabel,
      onOpen: () => openPage(context, hit.id, hit.title),
      onTick: checkable ? () => _watch.toggleDone(hit) : null,
    );
  }

  Widget _header(BuildContext context, int count, bool hasRows) {
    final c = context.colors;
    final ty = context.type;
    final base = ts(ty.code, c.textPrimary, mono: true);
    final query = _watch.query;
    final Widget field;
    if (_editing) {
      field = Focus(
        onKeyEvent: _onEditKey,
        child: PlainField(
          key: ValueKey('query-edit-${widget.block.node.id}'),
          controller: _edit,
          focusNode: _editFocus,
          style: base,
          placeholder: 'Search query',
          textInputAction: TextInputAction.done,
          onSubmitted: (_) => _applyEdit(),
        ),
      );
    } else {
      field = FocusFrame(
        onActivate: _startEdit,
        child: Hover(
          key: ValueKey('query-text-${widget.block.node.id}'),
          onTap: _startEdit,
          borderRadius: BorderRadius.circular(Radii.sm),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
            child: Text.rich(
              highlightQuery(
                query.isEmpty ? 'Search query' : query,
                base: base.copyWith(color: query.isEmpty ? c.textTertiary : c.textPrimary),
                tag: c.accentText,
                key: c.textTertiary,
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ),
      );
    }
    return Container(
      constraints: const BoxConstraints(minHeight: 44),
      padding: const EdgeInsets.fromLTRB(10, 6, 6, 6),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        border: !_collapsed && !_editing && hasRows
            ? Border(bottom: BorderSide(color: c.borderSubtle))
            : null,
      ),
      child: Row(
        children: [
          Icon20(Ico.search, size: 16, color: c.textTertiary),
          const SizedBox(width: 6),
          Expanded(child: field),
          const SizedBox(width: 6),
          QueryCountBadge(total: count),
          const SizedBox(width: 2),
          if (showInlineMenuItem) ...[
            FocusFrame(
              onActivate: () => _openOverflow(context),
              child: Hover(
                key: ValueKey('query-overflow-${widget.block.node.id}'),
                onTap: () => _openOverflow(context),
                borderRadius: BorderRadius.circular(Radii.sm),
                child: SizedBox(
                  width: 28,
                  height: 28,
                  child: Center(
                    child: Icon20(Ico.more, size: 16, color: c.iconSecondary),
                  ),
                ),
              ),
            ),
            const SizedBox(width: 2),
          ],
          FocusFrame(
            onActivate: () => setState(() => _collapsed = !_collapsed),
            child: Hover(
              key: ValueKey('query-toggle-${widget.block.node.id}'),
              onTap: () => setState(() => _collapsed = !_collapsed),
              borderRadius: BorderRadius.circular(Radii.sm),
              child: SizedBox(
                width: 28,
                height: 28,
                child: Center(
                  child: RotatedBox(
                    quarterTurns: _collapsed ? 1 : 0,
                    child: Icon20(Ico.chevronDown, size: 16, color: c.iconSecondary),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  Widget _editHint(BuildContext context) {
    final c = context.colors;
    final small = ts(context.type.uiSmall, c.textTertiary);
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 0, 12, 8),
      child: Row(
        children: [
          Expanded(child: Text('Updates as notes change', style: small)),
          Text('Enter apply · Esc cancel', style: small),
        ],
      ),
    );
  }
}
