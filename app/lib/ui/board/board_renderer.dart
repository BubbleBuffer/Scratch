// Shared board layout and interactions. Storage belongs to BoardSource.
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../ui.dart';
import 'board_source.dart';

/// Only stable IDs cross a drag boundary; no storage model enters the renderer.
class BoardCardDrag {
  const BoardCardDrag(this.cardId, this.fromColumnId);
  final String cardId, fromColumnId;
}

class BoardRenderer extends StatefulWidget {
  const BoardRenderer({
    super.key,
    required this.source,
    this.inline = false,
    this.gutter = 0,
    this.columnWidth = 276,
    this.columnGap = 12,
    this.inlinePageSize = 25,
    this.addCardLabel = 'New page',
    this.cardBuilder,
    this.columnLabelBuilder,
    this.columnMenu,
    this.trailing,
  });
  final BoardSource source;
  final bool inline;
  final double gutter, columnWidth, columnGap;
  final int inlinePageSize;
  final String addCardLabel;

  /// Optional rich card presentation. The renderer owns Enter/Space and
  /// pointer activation for noninteractive cards. An interactive custom card
  /// may keep its existing pointer handling; descendant keyboard focus is
  /// excluded so the board provides one focus stop and one open callback.
  final BoardCardBuilder? cardBuilder;
  final BoardColumnLabelBuilder? columnLabelBuilder;
  final BoardColumnMenu? columnMenu;
  final Widget? trailing;

  @override
  State<BoardRenderer> createState() => _BoardRendererState();
}

class _BoardRendererState extends State<BoardRenderer> {
  final _h = ScrollController();

  @override
  void dispose() {
    _h.dispose();
    super.dispose();
  }

  Future<void> _drop(BoardCardDrag drag, String to, String? after) async {
    if (after == drag.cardId) return;
    await widget.source.moveCard(drag.cardId, drag.fromColumnId, to, after);
  }

  void _moveColumn(String id, int direction) {
    final columns = widget.source.columns;
    final index = columns.indexWhere((column) => column.id == id);
    final next = index + direction;
    if (index < 0 || next < 0 || next >= columns.length) return;
    final after = direction > 0
        ? columns[next].id
        : (next == 0 ? null : columns[next - 1].id);
    widget.source.reorderColumn!(id, after);
  }

  Widget _card(
    BuildContext context,
    BoardColumn column,
    BoardCard card,
    bool lifted,
  ) {
    final c = context.colors;
    final firstLine =
        widget.cardBuilder == null && AppScope.maybeOf(context) != null
        ? context.watch<PagesStore>().meta(card.id)?.firstLine ?? ''
        : '';
    final body = widget.cardBuilder != null
        ? widget.cardBuilder!(context, column, card, lifted)
        : Hover(
            key: ValueKey('card-${card.title}'),
            onTap: () => widget.source.openCard(context, card),
            borderRadius: Radii.panel,
            lift: 2,
            background: c.bgPrimary,
            hoverBackground: Color.alphaBlend(c.bgHover, c.bgPrimary),
            border: Border.all(color: c.border),
            shadow: lifted ? c.seamHover : c.seam,
            hoverShadow: c.seamHover,
            child: Padding(
              padding: EdgeInsets.fromLTRB(
                10,
                context.mobile ? 10 : 8,
                10,
                context.mobile ? 10 : 8,
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    displayTitle(card.title, firstLine: firstLine),
                    style: ts(
                      context.mobile
                          ? const TypeSpec(15, 20, 500)
                          : const TypeSpec(14, 20, 500),
                      c.textPrimary,
                    ),
                  ),
                  for (final peek in card.peek)
                    Padding(
                      padding: const EdgeInsets.only(top: 6),
                      child: Text(
                        peek,
                        style: ts(context.type.ui, c.textSecondary),
                      ),
                    ),
                ],
              ),
            ),
          );
    if (lifted) return ExcludeFocus(child: body);
    return _BoardCardSurface(
      key: ValueKey('board-card-${card.id}'),
      onOpen: () => widget.source.openCard(context, card),
      child: body,
    );
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.source,
    builder: (context, _) {
      final columns = widget.source.columns;
      final cols = <Widget>[
        for (final column in columns)
          _BoardColumn(
            key: ValueKey('group-${column.id}'),
            state: this,
            group: column,
            width: widget.columnWidth,
          ),
        if (widget.trailing != null) widget.trailing!,
        if (widget.source.addColumn != null)
          SizedBox(
            width: widget.columnWidth,
            child: _BoardAddButton(
              key: const ValueKey('board-add-column'),
              label: 'New column',
              onTap: () => widget.source.addColumn!('New column'),
            ),
          ),
      ];
      final row = Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          for (var i = 0; i < cols.length; i++) ...[
            if (i > 0) SizedBox(width: widget.columnGap),
            cols[i],
          ],
        ],
      );
      return SingleChildScrollView(
        controller: _h,
        scrollDirection: Axis.horizontal,
        padding: EdgeInsets.only(
          left: widget.gutter,
          right: widget.gutter,
          top: 4,
        ),
        child: widget.inline
            ? row
            : LayoutBuilder(
                builder: (context, box) =>
                    SizedBox(height: box.maxHeight, child: row),
              ),
      );
    },
  );
}

/// One keyboard stop for any card presentation, including interactive cards.
class _BoardCardSurface extends StatefulWidget {
  const _BoardCardSurface({
    super.key,
    required this.onOpen,
    required this.child,
  });
  final VoidCallback onOpen;
  final Widget child;
  @override
  State<_BoardCardSurface> createState() => _BoardCardSurfaceState();
}

class _BoardCardSurfaceState extends State<_BoardCardSurface> {
  final _focus = FocusNode();
  bool _highlight = false;
  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => FocusableActionDetector(
    focusNode: _focus,
    onShowFocusHighlight: (highlight) => setState(() => _highlight = highlight),
    shortcuts: const {
      SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
      SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
    },
    actions: {
      ActivateIntent: CallbackAction<ActivateIntent>(
        onInvoke: (_) {
          widget.onOpen();
          return null;
        },
      ),
    },
    child: CustomPaint(
      foregroundPainter: _highlight
          ? _BoardFocusRing(context.colors.focus)
          : null,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onOpen,
        child: ExcludeFocus(child: widget.child),
      ),
    ),
  );
}

/// Same inset-independent focus outline used by the UI-kit Hover surface.
class _BoardFocusRing extends CustomPainter {
  const _BoardFocusRing(this.color);
  final Color color;
  @override
  void paint(Canvas canvas, Size size) => canvas.drawRRect(
    Radii.panel.toRRect((Offset.zero & size).inflate(3)),
    Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2,
  );
  @override
  bool shouldRepaint(_BoardFocusRing old) => old.color != color;
}

class _BoardColumn extends StatefulWidget {
  const _BoardColumn({
    super.key,
    required this.state,
    required this.group,
    required this.width,
  });
  final _BoardRendererState state;
  final BoardColumn group;
  final double width;

  @override
  State<_BoardColumn> createState() => _BoardColumnState();
}

class _BoardColumnState extends State<_BoardColumn> {
  bool _hover = false;
  bool _renaming = false;
  bool _dropEnd = false;
  late int _limit = b.widget.inlinePageSize;
  final _v = ScrollController();

  @override
  void dispose() {
    _v.dispose();
    super.dispose();
  }

  _BoardRendererState get b => widget.state;
  BoardSource get source => b.widget.source;
  BoardColumn get g => widget.group;

  /// The id to place a card after when dropped before row [i] (skipping
  /// the dragged card itself).
  String? _afterFor(int i, String dragged) {
    for (var j = i - 1; j >= 0; j--) {
      final r = source.cardAt(g.id, j);
      if (r == null) return null;
      if (r.id != dragged) return r.id;
    }
    return null;
  }

  String? _lastId(String dragged) {
    final rows = source
        .loadedCards(g.id)
        .where((r) => r.id != dragged)
        .toList();
    return rows.isEmpty ? null : rows.last.id;
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final tint = g.background ?? c.bgSecondary;
    final n = math.max(0, source.cardCount(g.id));
    final lazy = !b.widget.inline && n > 40;
    final count = b.widget.inline ? math.min(n, _limit) : n;
    final m = context.mobile;
    final label = g.keyLabel;

    Widget card(int i) {
      final r = source.cardAt(g.id, i);
      if (r == null) return const SizedBox(height: 60);
      final cardW = widget.width - 16;
      final body = b._card(context, g, r, false);
      final feedback = SizedBox(
        width: cardW,
        child: Transform.rotate(
          angle: 0.02,
          child: b._card(context, g, r, true),
        ),
      );
      final data = BoardCardDrag(r.id, g.id);
      final drag = m
          ? LongPressDraggable<BoardCardDrag>(
              data: data,
              feedback: feedback,
              childWhenDragging: Opacity(opacity: 0.35, child: body),
              child: body,
            )
          : Draggable<BoardCardDrag>(
              data: data,
              feedback: feedback,
              childWhenDragging: Opacity(opacity: 0.35, child: body),
              child: body,
            );
      return DragTarget<BoardCardDrag>(
        key: ValueKey('drop-${r.id}'),
        onWillAcceptWithDetails: (d) => d.data.cardId != r.id,
        onAcceptWithDetails: (d) =>
            b._drop(d.data, g.id, _afterFor(i, d.data.cardId)),
        builder: (context, cand, _) => Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (cand.isNotEmpty)
                Container(
                  height: 3,
                  margin: const EdgeInsets.only(bottom: 5),
                  decoration: BoxDecoration(
                    color: c.accent.withValues(alpha: 0.6),
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              drag,
            ],
          ),
        ),
      );
    }

    final header = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: SizedBox(
        height: m ? 44 : 34,
        child: Row(
          children: [
            const SizedBox(width: 2),
            Expanded(
              child: Row(
                children: [
                  if (_renaming)
                    Expanded(
                      child: _ColumnTitleEditor(
                        column: g,
                        onSubmitted: (title) async {
                          await source.renameColumn!(g.id, title);
                          if (mounted) setState(() => _renaming = false);
                        },
                      ),
                    )
                  else if (b.widget.columnLabelBuilder != null)
                    b.widget.columnLabelBuilder!(context, g)
                  else
                    Flexible(
                      child: Text(
                        g.title,
                        style: ts(
                          context.type.ui,
                          g.color ?? c.textPrimary,
                          weight: 500,
                        ),
                      ),
                    ),
                  const SizedBox(width: 8),
                  Text(
                    '${g.count}',
                    style: ts(
                      m ? const TypeSpec(15, 20, 400) : context.type.ui,
                      c.textTertiary,
                      weight: 400,
                    ),
                  ),
                ],
              ),
            ),
            if (_hover || m) ...[
              if (source.reorderColumn != null) ...[
                IconBtn(
                  Ico.chevronLeft,
                  key: ValueKey('board-column-left-${g.id}'),
                  size: 24,
                  glyph: 16,
                  color: c.iconTertiary,
                  semanticLabel: 'Move column left',
                  onTap: () => b._moveColumn(g.id, -1),
                ),
                IconBtn(
                  Ico.chevronRight,
                  key: ValueKey('board-column-right-${g.id}'),
                  size: 24,
                  glyph: 16,
                  color: c.iconTertiary,
                  semanticLabel: 'Move column right',
                  onTap: () => b._moveColumn(g.id, 1),
                ),
              ],
              if (b.widget.columnMenu != null ||
                  source.renameColumn != null ||
                  _canDelete)
                Builder(
                  builder: (bc) => IconBtn(
                    Ico.more,
                    key: ValueKey('group-menu-$label'),
                    size: 24,
                    glyph: 16,
                    color: c.iconTertiary,
                    semanticLabel: 'Group options',
                    onTap: () => _groupMenu(bc),
                  ),
                ),
              IconBtn(
                Ico.plus,
                key: ValueKey('group-add-$label'),
                size: 24,
                glyph: 16,
                color: c.iconTertiary,
                semanticLabel: b.widget.addCardLabel,
                onTap: () => source.addCard(context, g.id),
              ),
            ],
          ],
        ),
      ),
    );

    final newBtn = DragTarget<BoardCardDrag>(
      onWillAcceptWithDetails: (d) {
        setState(() => _dropEnd = true);
        return true;
      },
      onLeave: (_) => setState(() => _dropEnd = false),
      onAcceptWithDetails: (d) {
        setState(() => _dropEnd = false);
        b._drop(d.data, g.id, _lastId(d.data.cardId));
      },
      builder: (context, cand, _) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (_dropEnd)
            Container(
              height: 3,
              margin: const EdgeInsets.only(bottom: 5),
              decoration: BoxDecoration(
                color: c.accent.withValues(alpha: 0.6),
                borderRadius: BorderRadius.circular(2),
              ),
            ),
          if (b.widget.inline && n > count)
            _BoardLoadMore(
              onTap: () => setState(() => _limit += b.widget.inlinePageSize),
            ),
          _BoardAddButton(
            key: ValueKey('board-new-$label'),
            label: b.widget.addCardLabel,
            onTap: () => source.addCard(context, g.id),
            height: m ? 40 : 32,
          ),
        ],
      ),
    );

    final Widget cards;
    if (lazy) {
      cards = Flexible(
        child: ListView.builder(
          controller: _v,
          padding: EdgeInsets.zero,
          itemCount: count + 1,
          itemBuilder: (context, i) => i < count ? card(i) : newBtn,
        ),
      );
    } else {
      final list = Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [for (var i = 0; i < count; i++) card(i), newBtn],
      );
      cards = b.widget.inline
          ? list
          : Flexible(
              child: SingleChildScrollView(controller: _v, child: list),
            );
    }

    return Container(
      key: ValueKey('board-col-$label'),
      width: widget.width,
      padding: const EdgeInsets.fromLTRB(8, 2, 8, 6),
      decoration: BoxDecoration(
        color: tint,
        borderRadius: BorderRadius.circular(Radii.lg),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [header, cards],
      ),
    );
  }

  /// Delete needs a second column to receive the cards, so it is offered only
  /// while one other column remains.
  bool get _canDelete =>
      source.deleteColumn != null && source.columns.length > 1;

  void _groupMenu(BuildContext anchor) {
    if (b.widget.columnMenu != null) {
      b.widget.columnMenu!(anchor, g);
    } else if (_canDelete) {
      _columnMenu(anchor);
    } else if (source.renameColumn != null) {
      setState(() => _renaming = true);
    }
  }

  void _columnMenu(BuildContext anchor) {
    showPopover<void>(
      anchor,
      globalRectOf(anchor),
      (ctx) => MenuList(
        width: 220,
        children: [
          if (source.renameColumn != null)
            MenuItem(
              key: const ValueKey('group-rename'),
              label: 'Rename',
              onTap: () {
                Navigator.of(ctx).pop();
                setState(() => _renaming = true);
              },
            ),
          MenuItem(
            key: const ValueKey('group-delete'),
            label: 'Delete column',
            danger: true,
            onTap: () {
              Navigator.of(ctx).pop();
              _deleteColumn();
            },
          ),
        ],
      ),
    );
  }

  Future<void> _deleteColumn() async {
    final next = source.columns.firstWhere((column) => column.id != g.id);
    final ok = await showConfirm(
      context,
      title: 'Delete "${g.title}"?',
      message:
          'Its cards stay as notes and move to "${next.title}", '
          'the first remaining column.',
      confirmLabel: 'Delete column',
      danger: true,
    );
    if (!ok) return;
    await source.deleteColumn!(g.id);
  }
}

class _BoardAddButton extends StatelessWidget {
  const _BoardAddButton({
    super.key,
    required this.label,
    required this.onTap,
    this.height = 33,
  });
  final String label;
  final VoidCallback onTap;
  final double height;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      onTap: onTap,
      radius: Radii.sm,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: SizedBox(
        height: height,
        child: Row(
          children: [
            Icon20(Ico.plus, size: 14, color: c.iconTertiary),
            const SizedBox(width: 6),
            Text(
              label,
              style: ts(
                context.mobile ? const TypeSpec(15, 20, 400) : context.type.ui,
                c.textTertiary,
                weight: 400,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// "Load more" under an inline board's cards.
class _BoardLoadMore extends StatelessWidget {
  const _BoardLoadMore({required this.onTap});
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Hover(
      key: const ValueKey('load-more'),
      onTap: onTap,
      radius: Radii.sm,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: SizedBox(
        height: 33,
        child: Row(
          children: [
            Icon20(Ico.arrowDown, size: 14, color: c.iconTertiary),
            const SizedBox(width: 6),
            Text(
              'Load more',
              style: ts(context.type.ui, c.textTertiary, weight: 400),
            ),
          ],
        ),
      ),
    );
  }
}

class _ColumnTitleEditor extends StatefulWidget {
  const _ColumnTitleEditor({required this.column, required this.onSubmitted});
  final BoardColumn column;
  final ValueChanged<String> onSubmitted;
  @override
  State<_ColumnTitleEditor> createState() => _ColumnTitleEditorState();
}

class _ColumnTitleEditorState extends State<_ColumnTitleEditor> {
  late final _text = TextEditingController(text: widget.column.title);
  final _focus = FocusNode();
  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => PlainField(
    key: ValueKey('board-column-title-${widget.column.id}'),
    controller: _text,
    focusNode: _focus,
    autofocus: true,
    style: ts(context.type.ui, context.colors.textPrimary),
    placeholder: 'Column name',
    textInputAction: TextInputAction.done,
    onSubmitted: (title) => widget.onSubmitted(title.trim()),
  );
}
