// Form B's results panel: the chip's query results, opened from the chip.
// The rows, header and count are the card's (the same QueryWatch, the same
// row tile, the same tick and Undo). Desktop: a 420 px popover anchored to
// the chip. Phone: a bottom sheet with a grab handle over a scrim. On phone
// the Undo message sits above the footer inside the sheet, so it never
// overlaps the footer's buttons.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../services/legacy_host.dart' show openPage;
import '../../src/rust/api/types.dart';
import '../../ui/ui.dart';
import 'query_row.dart';
import 'query_toast.dart';
import 'query_watch.dart';

/// Rows the panel shows before "Show N more" (the mockup's panel).
const queryPanelRows = 6;

/// How long the phone's Undo message stays, as the toast's.
const _undoMessageFor = Duration(seconds: 6);

/// The panel's width on desktop.
const queryPanelWidth = 420.0;

/// Opens the panel for [watch]: a popover under [anchor] on desktop, a bottom
/// sheet on phone. [onOpenInSearch] and [onShowAsBlock] run after the panel
/// has closed.
Future<void> showQueryPanel(
  BuildContext context, {
  required QueryWatch watch,
  required Rect anchor,
  required VoidCallback onOpenInSearch,
  required VoidCallback onShowAsBlock,
}) {
  Widget panel(BuildContext panelContext, {required bool mobile}) =>
      QueryPanel(
        watch: watch,
        mobile: mobile,
        hostContext: context,
        onOpenInSearch: onOpenInSearch,
        onShowAsBlock: onShowAsBlock,
      );
  if (context.mobile) {
    return showSheet<void>(context, (ctx) => panel(ctx, mobile: true));
  }
  return showPopover<void>(
    context,
    anchor,
    (ctx) => SizedBox(width: queryPanelWidth, child: panel(ctx, mobile: false)),
  );
}

class QueryPanel extends StatefulWidget {
  const QueryPanel({
    super.key,
    required this.watch,
    required this.mobile,
    required this.hostContext,
    required this.onOpenInSearch,
    required this.onShowAsBlock,
  });

  final QueryWatch watch;

  /// Phone: a sheet, and the Undo message above the footer. Desktop: a
  /// popover, and the global toast.
  final bool mobile;

  /// The chip's context. It outlives the panel, so a row can open its page
  /// after the panel has closed.
  final BuildContext hostContext;
  final VoidCallback onOpenInSearch;
  final VoidCallback onShowAsBlock;

  @override
  State<QueryPanel> createState() => _QueryPanelState();
}

class _QueryPanelState extends State<QueryPanel> {
  late final StreamSubscription<Hit> _leftSub;
  Timer? _undoTimer;

  /// Phone: the task whose Undo message is showing, if any.
  Hit? _left;
  bool _expanded = false;

  @override
  void initState() {
    super.initState();
    widget.watch.addListener(_onWatch);
    _leftSub = widget.watch.rowsLeft.listen(_onLeft);
  }

  @override
  void didUpdateWidget(QueryPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.watch != widget.watch) {
      oldWidget.watch.removeListener(_onWatch);
      widget.watch.addListener(_onWatch);
    }
  }

  @override
  void dispose() {
    _undoTimer?.cancel();
    _leftSub.cancel();
    widget.watch.removeListener(_onWatch);
    super.dispose();
  }

  void _onWatch() {
    if (mounted) setState(() {});
  }

  void _onLeft(Hit hit) {
    if (!widget.mobile) {
      showQueryToast(
        context,
        'Marked done, so it left this list.',
        undo: () => widget.watch.setDone(hit, done: false),
      );
      return;
    }
    _undoTimer?.cancel();
    setState(() => _left = hit);
    _undoTimer = Timer(_undoMessageFor, _dismissMessage);
  }

  void _dismissMessage() {
    _undoTimer?.cancel();
    if (mounted) setState(() => _left = null);
  }

  void _undo(Hit hit) {
    _dismissMessage();
    widget.watch.setDone(hit, done: false);
  }

  /// Escape closes the panel; the chip keeps its place.
  KeyEventResult _onKey(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent) return KeyEventResult.ignored;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      Navigator.of(context).maybePop();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  void _close(VoidCallback then) {
    Navigator.of(context).maybePop();
    then();
  }

  @override
  Widget build(BuildContext context) {
    final watch = widget.watch;
    final c = context.colors;
    final ty = context.type;
    final visible = watch.visibleHits;
    final shown = _expanded
        ? visible
        : visible.take(queryPanelRows).toList();
    final more = visible.length - shown.length;
    final header = Container(
      padding: const EdgeInsets.fromLTRB(12, 6, 10, 6),
      constraints: const BoxConstraints(minHeight: 44),
      decoration: BoxDecoration(
        color: widget.mobile ? c.bgPrimary : c.bgSecondary,
        border: Border(bottom: BorderSide(color: c.borderSubtle)),
      ),
      child: Row(
        children: [
          Icon20(Ico.search, size: 16, color: c.textTertiary),
          const SizedBox(width: 6),
          Expanded(
            child: Text.rich(
              highlightQuery(
                watch.query,
                base: ts(ty.code, c.textPrimary, mono: true),
                tag: c.accentText,
                key: c.textTertiary,
              ),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const SizedBox(width: 6),
          QueryCountBadge(total: watch.count),
        ],
      ),
    );
    final Widget body;
    if (watch.query.trim().isEmpty) {
      body = const QueryEmptyLine(text: 'Type a query to see results.');
    } else if (watch.error != null) {
      body = const QueryEmptyLine(text: "Couldn't run this query.");
    } else if (watch.ready && visible.isEmpty) {
      body = QueryEmptyLine(text: watch.wording.emptyLine);
    } else {
      body = Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var i = 0; i < shown.length; i++)
            DecoratedBox(
              decoration: BoxDecoration(
                border: i == 0
                    ? null
                    : Border(top: BorderSide(color: c.borderSubtle)),
              ),
              child: _row(shown[i]),
            ),
          if (more > 0)
            Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(8, 0, 8, 6),
                child: TextBtn(
                  'Show $more more',
                  key: const ValueKey('query-panel-more'),
                  onTap: () => setState(() => _expanded = true),
                ),
              ),
            ),
        ],
      );
    }
    final left = _left;
    final message = widget.mobile && left != null
        ? _UndoMessage(
            hit: left,
            onUndo: () => _undo(left),
          )
        : null;
    final footer = Container(
      padding: EdgeInsets.fromLTRB(
        widget.mobile ? 14 : 12,
        8,
        widget.mobile ? 14 : 12,
        10,
      ),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: c.borderSubtle)),
      ),
      child: Row(
        children: [
          TextBtn(
            'Open in Search',
            key: const ValueKey('query-panel-open-search'),
            onTap: () => _close(widget.onOpenInSearch),
          ),
          const SizedBox(width: 18),
          TextBtn(
            'Show as a block',
            key: const ValueKey('query-panel-as-block'),
            onTap: () => _close(widget.onShowAsBlock),
          ),
        ],
      ),
    );
    final content = Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        header,
        body,
        ?message,
        footer,
      ],
    );
    final panel = Focus(
      autofocus: true,
      onKeyEvent: _onKey,
      child: Semantics(
        container: true,
        label: '${watch.wording.chipLabel} search results',
        child: content,
      ),
    );
    if (widget.mobile) {
      return SheetSurface(children: [panel]);
    }
    return Container(
      key: const ValueKey('query-panel'),
      decoration: BoxDecoration(
        color: c.bgPrimary,
        border: Border.all(color: c.border),
        borderRadius: Radii.panel,
        boxShadow: c.seamHover,
      ),
      clipBehavior: Clip.antiAlias,
      child: panel,
    );
  }

  Widget _row(Hit hit) {
    final watch = widget.watch;
    final info = watch.infoOf(hit);
    return QueryRowTile(
      hit: hit,
      done: watch.isDone(hit),
      pageIcon: info?.icon,
      statusLabel: info?.statusLabel,
      onOpen: () =>
          _close(() => openPage(widget.hostContext, hit.id, hit.title)),
      onTick: watch.isTaskRow(hit) ? () => watch.toggleDone(hit) : null,
    );
  }
}

/// The phone's Undo message, drawn above the panel's footer.
class _UndoMessage extends StatelessWidget {
  const _UndoMessage({required this.hit, required this.onUndo});

  final Hit hit;
  final VoidCallback onUndo;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      key: const ValueKey('query-panel-message'),
      padding: const EdgeInsets.fromLTRB(14, 8, 14, 8),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: c.borderSubtle)),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              'Marked done, so it left this list.',
              style: ts(context.type.ui, c.textPrimary),
            ),
          ),
          TextBtn(
            'Undo',
            key: const ValueKey('query-panel-undo'),
            onTap: onUndo,
          ),
        ],
      ),
    );
  }
}
