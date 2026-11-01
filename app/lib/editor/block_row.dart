// One block of the page: the block type's decoration (marker, bar, box)
// around its text (an EditableText with the app's own gestures), and on
// desktop the hover handles ("+" and "⋮⋮") 44 px left of the text. What a
// kind looks like comes from the block registry. Rows inside a column are
// built by the column (see PageEditorState.nestedBlocks) with their own
// geometry and a base depth.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../blocks/columns.dart';
import '../core/models.dart';
import '../registry/addon_registry.dart' show InlineMarkRef;
import '../registry/block_registry.dart';
import '../state/app_state.dart';
import '../ui/block_colors.dart';
import '../ui/ui.dart';
import 'model.dart';
import 'page_screen.dart';
import 'rich_controller.dart';

/// Horizontal geometry of the text column inside the scroll view.
class Geo {
  const Geo(this.textLeft, this.textWidth);
  final double textLeft, textWidth;
}

/// What a feature adds to the blocks of a page (the conflict review uses it).
/// Everything is drawn inside the text column, never over the handles or the
/// gutter. It notifies when its content changes, and the rows rebuild then.
abstract class BlockDecor implements Listenable {
  /// The tint behind block [id]'s content, or null for none.
  Color? tone(BuildContext context, String id);

  /// A widget above block [id]'s content, or null. This is the one wrapper
  /// the hooks have: a label sits above the content and is not part of it, so
  /// the handles (which are positioned on the content) stay aligned.
  Widget? above(BuildContext context, String id);

  /// A widget below block [id]'s content, or null.
  Widget? below(BuildContext context, String id);
}

class BlockRow extends StatefulWidget {
  const BlockRow({
    super.key,
    required this.node,
    required this.ed,
    required this.geo,
    this.baseDepth = 0,
  });
  final Node node;
  final PageEditorState ed;
  final Geo geo;

  /// Depth of the first level of rows inside a column (indent starts there).
  final int baseDepth;

  @override
  State<BlockRow> createState() => BlockRowState();
}

class BlockRowState extends State<BlockRow>
    with AutomaticKeepAliveClientMixin
    implements TextSelectionGestureDetectorBuilderDelegate {
  late _Gestures _gestures;
  bool _hover = false;
  late FocusNode _focus = widget.ed.fn(widget.node);

  Node get node => widget.node;
  PageEditorState get ed => widget.ed;

  @override
  GlobalKey<EditableTextState> get editableTextKey => ed.editableKey(node);
  @override
  bool get forcePressEnabled => false;
  @override
  bool get selectionEnabled => true;

  @override
  bool get wantKeepAlive => _focus.hasFocus;

  @override
  void initState() {
    super.initState();
    _gestures = _Gestures(this);
    _focus.addListener(_focusChanged);
    ed.register(this);
  }

  @override
  void didUpdateWidget(BlockRow oldWidget) {
    final old = oldWidget;
    super.didUpdateWidget(oldWidget);
    if (old.node != node) {
      ed.unregister(old.node.id, this);
      ed.controllerOf(old.node)?.releaseInline();
      _focus.removeListener(_focusChanged);
      _focus = ed.fn(node);
      _focus.addListener(_focusChanged);
      ed.register(this);
    }
  }

  @override
  void dispose() {
    _focus.removeListener(_focusChanged);
    ed.unregister(node.id, this);
    ed.controllerOf(node)?.releaseInline();
    super.dispose();
  }

  /// A tap on an add-on's inline chip at [offset] opens its panel, anchored
  /// to the chip. Returns true when it did.
  bool openInlineAt(RenderEditable re, int offset) {
    final c = ed.ctl(node);
    for (final mark in c.marks) {
      final provider = c.inlineProviderOf(mark);
      final state = c.inlineStateOf(mark);
      if (provider == null || state == null) continue;
      if (mark.start > offset || offset >= mark.end) continue;
      // As a page mention: Ctrl-click in the focused block places the caret.
      if (ed.focusedId == node.id &&
          !ed.readOnly &&
          isCommandKeyPressed) {
        return false;
      }
      final boxes = re.getBoxesForSelection(
        TextSelection(baseOffset: mark.start, extentOffset: mark.end),
      );
      if (boxes.isEmpty) return false;
      final box = boxes.first.toRect();
      provider.open(
        context,
        re.localToGlobal(box.topLeft) & box.size,
        InlineMarkRef(
          payload: mark.url,
          state: state,
          host: ed,
          block: node,
          range: TextRange(start: mark.start, end: mark.end),
        ),
      );
      return true;
    }
    return false;
  }

  void _focusChanged() {
    if (!mounted) return;
    updateKeepAlive();
    ed.onFocus(node, _focus.hasFocus);
    setState(() {});
  }

  RenderEditable? get renderEditable =>
      editableTextKey.currentState?.renderEditable;

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final c = context.colors;
    final ty = context.type;
    final mobile = context.mobile;
    final n = node;
    final type = blockRegistry.of(n.kind);
    final geo = widget.geo;
    final indent = (n.depth - widget.baseDepth) * Layout.indent;
    var style = type.textStyle(n, c, ty);
    if (ed.smallText && style.fontSize != null) {
      style = style.copyWith(fontSize: style.fontSize! * 14 / 16);
    }
    final color = type.colorable && !type.paintsColor ? n.color : null;
    final textColor = paletteText(color, c);
    if (textColor != null) style = style.copyWith(color: textColor);
    final fill = isBackgroundColor(color) ? paletteBackground(color, c) : null;
    final lineH = (style.height ?? 1.5) * (style.fontSize ?? 16);
    final selected = ed.isSelected(n);
    final top = type.topMargin(mobile);
    final b = BlockContext(
      context: context,
      node: n,
      host: ed,
      style: style,
      lineHeight: lineH,
    );
    final embedded = type.children == ChildMode.embedded;

    Widget box;
    if (!type.editable) {
      box = GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => ed.tapBlock(n),
        onLongPress: mobile ? () => ed.onLongPress(n) : null,
        child: type.body?.call(b) ?? const SizedBox(height: 24),
      );
    } else {
      final text = _text(context, n, type, style);
      box = type.decorate?.call(b, text) ?? text;
      // 28 px for one line of body text: 24 + 2 + 2 on desktop, 22 + 3 + 3 on mobile.
      box = Padding(
        padding: EdgeInsets.symmetric(vertical: mobile ? 3 : 2),
        child: box,
      );
    }

    box = Container(
      decoration: BoxDecoration(
        color: fill,
        borderRadius: BorderRadius.circular(Radii.sm),
      ),
      foregroundDecoration: selected || ed.highlightedBlocks.contains(n.id)
          ? BoxDecoration(
              color: c.bgSelectedBlock,
              borderRadius: BorderRadius.circular(Radii.sm),
            )
          : null,
      padding: const EdgeInsets.symmetric(horizontal: Layout.blockPaddingX),
      child: box,
    );

    if (ed.selectMode && mobile) {
      box = GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => ed.toggleSelected(n),
        child: AbsorbPointer(child: box),
      );
    }

    final shift = embedded && !mobile ? columnGutter : 0.0;
    final showHandles =
        !mobile &&
        !embedded &&
        !ed.readOnly &&
        (_hover || ed.menuBlockId == n.id) &&
        ed.dragNode == null;
    final handleTop =
        type.handleTop?.call(b) ?? (mobile ? 3 : 2) + (lineH - 24) / 2;
    // Where the content's box starts, and where its text starts inside it.
    final boxLeft = math.max(
      0.0,
      geo.textLeft + indent - Layout.blockPaddingX - shift,
    );
    final textLeft = boxLeft + Layout.blockPaddingX;

    Widget row(BuildContext ctx, BlockDecor? feature) {
      final tone = feature?.tone(ctx, n.id);
      final above = feature?.above(ctx, n.id);
      final below = feature?.below(ctx, n.id);
      // The feature's labels sit in the text column, and the tone covers the
      // content box only. Handles stay with the content, not the labels.
      Widget aside(Widget w) => Padding(
        padding: EdgeInsets.only(left: textLeft),
        child: SizedBox(
          width: math.max(0, geo.textWidth - indent + shift),
          child: w,
        ),
      );
      return MouseRegion(
        onEnter: (_) => mobile ? null : setState(() => _hover = true),
        onExit: (_) => mobile ? null : setState(() => _hover = false),
        child: Padding(
          padding: EdgeInsets.only(
            top: top + Layout.blockRowGap / 2,
            bottom: Layout.blockRowGap / 2,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              if (above != null) aside(above),
              Stack(
                clipBehavior: Clip.none,
                children: [
                  Padding(
                    // Rows inside a stacked column (mobile) start at 0: clamp.
                    padding: EdgeInsets.only(left: boxLeft),
                    child: SizedBox(
                      width: math.max(
                        0,
                        geo.textWidth - indent + 2 * Layout.blockPaddingX + shift,
                      ),
                      child: tone == null
                          ? box
                          : DecoratedBox(
                              decoration: BoxDecoration(
                                color: tone,
                                borderRadius: BorderRadius.circular(Radii.sm),
                              ),
                              child: box,
                            ),
                    ),
                  ),
                  if (!mobile && !embedded)
                    Positioned(
                      left: geo.textLeft + indent + Layout.handleOffset,
                      top: handleTop,
                      child: AnimatedOpacity(
                        opacity: showHandles ? 1 : 0,
                        duration: Motion.of(context).fast,
                        child: IgnorePointer(
                          ignoring: !showHandles,
                          child: _handles(context),
                        ),
                      ),
                    ),
                ],
              ),
              if (below != null) aside(below),
            ],
          ),
        ),
      );
    }

    final decor = ed.blockDecor;
    if (decor == null) return row(context, null);
    return ListenableBuilder(
      listenable: decor,
      builder: (ctx, _) => row(ctx, decor),
    );
  }

  Widget _handles(BuildContext context) {
    final c = context.colors;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Hover(
          key: ValueKey('add-${node.id}'),
          onTap: () => ed.addBlockBelow(node),
          radius: Radii.sm,
          child: SizedBox(
            width: 18,
            height: 24,
            child: Center(
              child: Icon20(Ico.plus, size: 16, color: c.iconTertiary),
            ),
          ),
        ),
        GestureDetector(
          onPanStart: (d) => ed.dragStart(node, d.globalPosition),
          onPanUpdate: (d) => ed.dragUpdate(d.globalPosition),
          onPanEnd: (_) => ed.dragEnd(),
          onPanCancel: ed.dragCancel,
          child: Hover(
            key: ValueKey('grip-${node.id}'),
            cursor: SystemMouseCursors.grab,
            onTap: () {
              final box = context.findRenderObject() as RenderBox;
              final o = box.localToGlobal(Offset.zero);
              ed.openBlockMenu(node, o & box.size);
            },
            radius: Radii.sm,
            child: SizedBox(
              width: 18,
              height: 24,
              child: Center(
                child: Icon20(Ico.grip, size: 16, color: c.iconTertiary),
              ),
            ),
          ),
        ),
      ],
    );
  }

  Widget _text(BuildContext context, Node n, BlockType type, TextStyle style) {
    final c = context.colors;
    final ctl = ed.ctl(n);
    ctl.colors = c;
    ctl.inlineMarks = ed.inlineMarks;
    ctl.inlineContext = context;
    ctl.syncInline();
    final pages = context.app.pages;
    ctl.mentionGap = mentionGapFor(style.fontSize ?? 16);
    ctl.mentionIcon = (id) => pages.meta(id)?.icon;
    ctl.pageLinkFill = (id) {
      final ancestors = pages.ancestors(id);
      return spacePatch(
        ancestors.isEmpty ? pages.meta(id)?.color : ancestors.first.color,
        c.dark ? Brightness.dark : Brightness.light,
      )?.fill;
    };
    final focused = _focus.hasFocus;
    ctl.dateChips = n.kind == Kinds.todo && !focused;
    ctl.todoChecked = n.checked;
    final ph = type.placeholder;
    final showPh = ph.isNotEmpty && (type.alwaysShowPlaceholder || focused);
    final editable = EditableText(
      key: editableTextKey,
      controller: ctl,
      focusNode: _focus,
      readOnly: ed.readOnly,
      style: style,
      cursorColor: c.textPrimary,
      backgroundCursorColor: c.textTertiary,
      selectionColor: c.textSelection,
      cursorWidth: 1.5,
      maxLines: null,
      keyboardType: TextInputType.multiline,
      textInputAction: TextInputAction.newline,
      selectionControls: noteSelectionControls,
      rendererIgnoresPointer: true,
      showSelectionHandles: context.mobile,
      enableInteractiveSelection: true,
      contextMenuBuilder: null,
      onChanged: (v) => ed.onChanged(n, ctl.value),
      onSelectionChanged: (sel, cause) => ed.onSelectionChanged(n, sel, cause),
      textHeightBehavior: const TextHeightBehavior(
        leadingDistribution: TextLeadingDistribution.even,
      ),
    );
    return _gestures.buildGestureDetector(
      behavior: HitTestBehavior.translucent,
      child: ValueListenableBuilder<TextEditingValue>(
        valueListenable: ctl,
        child: CustomPaint(
          painter: MarkPainter(
            controller: ctl,
            editable: editableTextKey,
            colors: c,
            textScaler: MediaQuery.textScalerOf(context),
            icons: pages,
          ),
          child: editable,
        ),
        builder: (context, v, child) => Stack(
          children: [
            if (showPh && v.text.isEmpty)
              Positioned.fill(
                child: IgnorePointer(
                  child: Text(
                    ph,
                    maxLines: 1,
                    overflow: TextOverflow.clip,
                    style: style.copyWith(
                      color: (type.paintsColor || isBackgroundColor(n.color))
                          ? c.textPrimary
                          : c.textSecondary,
                      decoration: TextDecoration.none,
                    ),
                  ),
                ),
              ),
            // A mention at the very start has no character before it to
            // leave room for its icon: the text moves over instead.
            Padding(
              padding: EdgeInsets.only(
                left:
                    ctl.marks.any(
                      (m) =>
                          m.kind == BMarkKind.pageLink &&
                          m.start == 0 &&
                          m.end > 0,
                    )
                    ? ctl.mentionGap
                    : 0,
              ),
              child: child,
            ),
          ],
        ),
      ),
    );
  }
}

class _Gestures extends TextSelectionGestureDetectorBuilder {
  _Gestures(this.row) : super(delegate: row);
  final BlockRowState row;

  @override
  void onSingleLongTapStart(LongPressStartDetails details) {
    if (row.context.mobile) {
      row.ed.onLongPress(row.node);
      return;
    }
    super.onSingleLongTapStart(details);
  }

  @override
  void onSingleLongTapMoveUpdate(LongPressMoveUpdateDetails details) {
    if (row.context.mobile) return;
    super.onSingleLongTapMoveUpdate(details);
  }

  @override
  void onSingleLongTapEnd(LongPressEndDetails details) {
    if (row.context.mobile) return;
    super.onSingleLongTapEnd(details);
  }

  @override
  void onSingleTapUp(TapDragUpDetails details) {
    final ed = row.ed;
    if (HardwareKeyboard.instance.isShiftPressed &&
        ed.extendSelectionTo(row.node)) {
      return;
    }
    // A page mention opens its page.
    final re = row.renderEditable;
    if (re != null) {
      final pos = re.getPositionForPoint(details.globalPosition);
      if (ed.openMentionAt(row.node, pos.offset)) return;
      if (row.openInlineAt(re, pos.offset)) return;
    }
    ed.clearBlockSelection();
    super.onSingleTapUp(details);
    row.editableTextKey.currentState?.requestKeyboard();
  }
}
