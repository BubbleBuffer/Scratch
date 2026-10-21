// Editor menus: the `/` menu, the desktop selection toolbar, the mobile
// keyboard toolbar, the desktop block menu, the mobile block sheet and the
// link box. Built from the UI kit; block kinds come from the registry.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/block_registry.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import '../features/ai/ask_ai.dart' show AiMark;
import 'color_menu.dart';

/// A block type's icon: its [Ico], or its short text ("H1").
class KindIcon extends StatelessWidget {
  const KindIcon(this.type, {super.key, this.size = 20, this.color});
  final BlockType type;
  final double size;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final col = color ?? context.colors.iconPrimary;
    final ico = type.icon;
    if (ico != null) return Icon20(ico, size: size, color: col);
    final ix = type.iconX;
    if (ix != null) return IconX(ix, size: size, color: col);
    final t = type.iconText ?? '?';
    // "▸H1" (toggle headings): a small triangle before the level.
    final toggle = t.startsWith('▸');
    return SizedBox(
      width: size,
      height: size,
      child: Center(
        child: FittedBox(
          fit: BoxFit.scaleDown,
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            if (toggle) IconX(Ix.triangle, size: size * 0.45, color: col),
            Text(toggle ? t.substring(1) : t,
                maxLines: 1, softWrap: false, style: tsRaw(size * (toggle ? 0.5 : 0.6), size * 0.7, 600, col)),
          ]),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// `/` menu

class SlashMenu extends StatelessWidget {
  const SlashMenu({
    super.key,
    required this.types,
    required this.active,
    required this.onPick,
    required this.onHover,
    this.title = 'Basic blocks',
    this.maxHeight = 400,
    this.grouped = true,
  });
  final List<BlockType> types;

  /// Group headers ("Basic blocks", "Media", "Advanced") instead of [title].
  final bool grouped;
  final int active;
  final ValueChanged<String> onPick;
  final ValueChanged<int> onHover;
  final String title;
  final double maxHeight;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final ty = context.type;
    return MenuSurface(
      width: m ? null : 324,
      padding: const EdgeInsets.all(6),
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: maxHeight - 12),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (types.isEmpty || !grouped) MenuHeader(types.isEmpty ? 'No results' : title),
              for (var i = 0; i < types.length; i++) ...[
                if (grouped && (i == 0 || types[i - 1].group != types[i].group)) ...[
                  if (i > 0) const MenuSeparator(),
                  MenuHeader(types[i].group),
                ],
                MenuItem(
                  key: ValueKey('slash-${types[i].kind}'),
                  label: types[i].menuLabel,
                  leading: KindIcon(types[i], size: m ? 22 : 20),
                  selected: i == active,
                  onTap: () => onPick(types[i].kind),
                  onHover: () => onHover(i),
                  trailing: Text(types[i].shortcut, style: ts(ty.uiSmall, c.textTertiary, weight: 400)),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Selection toolbar (desktop)

class SelectionToolbar extends StatelessWidget {
  const SelectionToolbar({
    super.key,
    required this.kind,
    required this.active,
    required this.onMark,
    required this.onLink,
    required this.onTurnInto,
    this.onColor,
    this.onAskAgent,
    this.onAskAi,
  });
  final String kind;
  final Set<BMarkKind> active;
  final ValueChanged<BMarkKind> onMark;
  final VoidCallback onLink;
  final VoidCallback onTurnInto;
  final VoidCallback? onAskAgent;

  /// Ask AI about the selection (its answer goes after the block).
  final VoidCallback? onAskAi;

  /// Opens the colour menu under the given anchor.
  final ValueChanged<BuildContext>? onColor;

  static const height = 36.0;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget glyph(String s, BMarkKind k, {FontStyle? style, TextDecoration? deco, int weight = 600}) {
      final on = active.contains(k);
      return Hover(
        key: ValueKey('fmt-${k.name}'),
        onTap: () => onMark(k),
        radius: Radii.sm,
        child: SizedBox(
          width: 28,
          height: 28,
          child: Center(
            child: Text(s,
                style: tsRaw(15, 20, weight, on ? c.accentText : c.iconPrimary)
                    .copyWith(fontStyle: style, decoration: deco, decorationColor: on ? c.accentText : c.iconPrimary)),
          ),
        ),
      );
    }

    return Container(
      height: height,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      decoration: BoxDecoration(
        color: c.bgElevated,
        borderRadius: BorderRadius.circular(8),
        boxShadow: c.shadowMenu,
      ),
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        Hover(
          key: const ValueKey('fmt-turn-into'),
          onTap: onTurnInto,
          radius: Radii.sm,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: SizedBox(
            height: 28,
            child: Row(children: [
              Text(blockRegistry.of(kind).label, style: ts(TypeScale.desktop.ui, c.textPrimary)),
              const SizedBox(width: 4),
              Icon20(Ico.chevronDown, size: 12, color: c.iconTertiary),
            ]),
          ),
        ),
        const VLine(),
        IconBtn(Ico.link,
            key: const ValueKey('fmt-link'),
            onTap: onLink,
            glyph: 16,
            radius: Radii.sm,
            semanticLabel: active.contains(BMarkKind.link) ? 'Edit link' : 'Add link',
            color: active.contains(BMarkKind.link) ? c.accent : c.iconPrimary),
        const VLine(),
        glyph('B', BMarkKind.bold, weight: 700),
        glyph('i', BMarkKind.italic, style: FontStyle.italic, weight: 500),
        glyph('U', BMarkKind.underline, deco: TextDecoration.underline, weight: 500),
        glyph('S', BMarkKind.strike, deco: TextDecoration.lineThrough, weight: 500),
        Semantics(
          button: true,
          label: 'Code',
          child: AppTooltip(
            label: 'Code',
            child: Hover(
              key: const ValueKey('fmt-code'),
              onTap: () => onMark(BMarkKind.code),
              radius: Radii.sm,
              child: SizedBox(
                width: 28,
                height: 28,
                child: Center(
                    child: Icon20(Ico.code,
                        size: 16, color: active.contains(BMarkKind.code) ? c.accent : c.iconPrimary)),
              ),
            ),
          ),
        ),
        if (onColor != null) ...[
          const VLine(),
          Builder(
            builder: (anchor) => Hover(
              key: const ValueKey('fmt-color'),
              onTap: () => onColor!(anchor),
              radius: Radii.sm,
              padding: const EdgeInsets.symmetric(horizontal: 4),
              child: SizedBox(
                height: 28,
                child: Row(children: [
                  Text('A',
                      style: tsRaw(15, 20, 500, c.iconPrimary)
                          .copyWith(decoration: TextDecoration.underline, decorationColor: c.iconPrimary)),
                  const SizedBox(width: 2),
                  Icon20(Ico.chevronDown, size: 12, color: c.iconTertiary),
                ]),
              ),
            ),
          ),
        ],
        if (onAskAgent != null || onAskAi != null) const VLine(),
        if (onAskAi != null)
          AppTooltip(
            label: 'Ask AI',
            child: Semantics(
              button: true,
              label: 'Ask AI',
              child: Hover(
                key: const ValueKey('fmt-ask-ai'),
                onTap: onAskAi,
                radius: Radii.sm,
                child: const SizedBox(width: 28, height: 28, child: Center(child: AiMark(size: 16))),
              ),
            ),
          ),
        if (onAskAgent != null)
          IconBtn(Ico.comment,
              key: const ValueKey('fmt-ask-agent'),
              onTap: onAskAgent,
              glyph: 16,
              radius: Radii.sm,
              semanticLabel: 'Ask agent'),
      ]),
    );
  }
}

// ---------------------------------------------------------------------------
// Keyboard toolbar (mobile)

/// Which row of the keyboard toolbar shows.
enum KbMode { main, format, more }

/// The bar above the soft keyboard. Each row fits a 375px screen without
/// scrolling: eight 40px buttons, a divider and the 44px hide button.
class KeyboardToolbar extends StatelessWidget {
  const KeyboardToolbar({
    super.key,
    required this.mode,
    required this.active,
    required this.kind,
    required this.canUndo,
    required this.canRedo,
    required this.onAdd,
    required this.onMode,
    required this.onTurnInto,
    required this.onKind,
    required this.onIndent,
    required this.onOutdent,
    required this.onUndo,
    required this.onRedo,
    required this.onHide,
    required this.onMark,
    required this.onLink,
    required this.onMention,
    required this.onColor,
  });
  final KbMode mode;
  final Set<BMarkKind> active;
  final String kind;
  final bool canUndo, canRedo;
  final VoidCallback onAdd, onTurnInto, onIndent, onOutdent, onUndo, onRedo, onHide, onLink, onMention, onColor;
  final ValueChanged<KbMode> onMode;
  final ValueChanged<String> onKind;
  final ValueChanged<BMarkKind> onMark;

  static const button = 40.0;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    // [tip] is the label of an icon-only box; text boxes pass none.
    Widget box(String key, Widget child, VoidCallback? onTap, {bool on = false, String? tip}) {
      final hover = Hover(
        key: ValueKey('kb-$key'),
        onTap: onTap,
        selected: on,
        selectedColor: c.bgPressed,
        radius: Radii.md,
        child: SizedBox(width: button, height: 44, child: Center(child: child)),
      );
      if (tip == null) return hover;
      return Semantics(button: true, label: tip, child: AppTooltip(label: tip, child: hover));
    }

    Widget btn(Ico i, VoidCallback? onTap, String key, {bool on = false, String? tip}) => box(
        key, Icon20(i, size: 22, color: onTap == null ? c.textDisabled : c.iconSecondary), onTap, on: on, tip: tip);
    Widget btnX(Ix i, VoidCallback? onTap, String key, {bool on = false, String? tip}) => box(
        key, IconX(i, size: 22, color: onTap == null ? c.textDisabled : c.iconSecondary), onTap, on: on, tip: tip);
    Widget glyph(String s, BMarkKind k, {FontStyle? style, TextDecoration? deco, int weight = 600}) {
      final on = active.contains(k);
      return box(
        k.name,
        Text(s,
            style: tsRaw(18, 22, weight, on ? c.textPrimary : c.iconSecondary)
                .copyWith(fontStyle: style, decoration: deco, decorationColor: c.iconSecondary)),
        () => onMark(k),
        on: on,
      );
    }

    final items = switch (mode) {
      KbMode.format => <Widget>[
          btn(Ico.chevronLeft, () => onMode(KbMode.main), 'back', tip: 'Back'),
          glyph('B', BMarkKind.bold, weight: 700),
          glyph('i', BMarkKind.italic, style: FontStyle.italic, weight: 500),
          glyph('U', BMarkKind.underline, deco: TextDecoration.underline, weight: 500),
          glyph('S', BMarkKind.strike, deco: TextDecoration.lineThrough, weight: 500),
          btn(Ico.code, () => onMark(BMarkKind.code), 'code', on: active.contains(BMarkKind.code), tip: 'Code'),
          btn(Ico.link, onLink, 'link',
              on: active.contains(BMarkKind.link), tip: active.contains(BMarkKind.link) ? 'Edit link' : 'Add link'),
          btnX(Ix.color, onColor, 'color', tip: 'Text color'),
        ],
      KbMode.more => <Widget>[
          btn(Ico.chevronLeft, () => onMode(KbMode.main), 'back', tip: 'Back'),
          btn(Ico.indent, onIndent, 'indent', tip: 'Indent'),
          btn(Ico.outdent, onOutdent, 'outdent', tip: 'Outdent'),
          btn(Ico.numbers, () => onKind(Kinds.numbered), 'numbers',
              on: kind == Kinds.numbered, tip: 'Numbered list'),
          btnX(Ix.triangle, () => onKind(Kinds.toggle), 'toggle', on: kind == Kinds.toggle, tip: 'Toggle list'),
          btn(Ico.quote, () => onKind(Kinds.quote), 'quote', on: kind == Kinds.quote, tip: 'Quote'),
          btnX(Ix.mention, onMention, 'mention', tip: 'Mention'),
          btnX(Ix.color, onColor, 'block-color', tip: 'Block color'),
        ],
      KbMode.main => <Widget>[
          btn(Ico.plus, onAdd, 'add', tip: 'Add block'),
          box('format', Text('Aa', style: tsRaw(17, 22, 500, c.iconSecondary)), () => onMode(KbMode.format)),
          btn(Ico.turnInto, onTurnInto, 'turn-into', tip: 'Turn into'),
          btn(Ico.todo, () => onKind(Kinds.todo), 'todo', on: kind == Kinds.todo, tip: 'To-do list'),
          btn(Ico.bullets, () => onKind(Kinds.bulleted), 'bullets', on: kind == Kinds.bulleted, tip: 'Bulleted list'),
          btn(Ico.undo, canUndo ? onUndo : null, 'undo', tip: 'Undo'),
          btnX(Ix.redo, canRedo ? onRedo : null, 'redo', tip: 'Redo'),
          btn(Ico.more, () => onMode(KbMode.more), 'more', tip: 'More'),
        ],
    };

    return Container(
      height: Layout.keyboardToolbarHeight,
      decoration: BoxDecoration(
        color: c.bgElevated,
        border: Border(top: BorderSide(color: c.borderSubtle)),
      ),
      child: Row(children: [
        Expanded(
          child: ListView(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.symmetric(horizontal: 2),
            children: items,
          ),
        ),
        Container(width: 1, height: 24, color: c.border),
        IconBtn(Ico.keyboardHide,
            key: const ValueKey('kb-hide'),
            onTap: onHide,
            size: 44,
            glyph: 22,
            radius: Radii.md,
            color: c.iconSecondary,
            semanticLabel: 'Hide keyboard'),
      ]),
    );
  }
}

// ---------------------------------------------------------------------------
// Block menu (desktop, from the ⋮⋮ handle)

enum BlockAction { turnInto, moveUp, moveDown, duplicate, delete, color, select, copy, setDate }

class BlockMenu extends StatefulWidget {
  const BlockMenu({
    super.key,
    required this.kind,
    required this.onAction,
    required this.onTurnInto,
    required this.onColor,
    this.onAskAgent,
    this.onAskAi,
    this.count = 1,
    this.color,
  });
  final String kind;
  final ValueChanged<BlockAction> onAction;
  final ValueChanged<String> onTurnInto;
  final ValueChanged<String> onColor;
  final VoidCallback? onAskAgent;

  /// Ask AI about the block(s); the answer goes after them.
  final VoidCallback? onAskAi;

  /// How many blocks the menu acts on.
  final int count;

  /// The block's colour (ticked in the colour menu).
  final String? color;

  @override
  State<BlockMenu> createState() => _BlockMenuState();
}

class _BlockMenuState extends State<BlockMenu> {
  int _sub = 0; // 1: turn into, 2: colour

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    if (_sub == 1) {
      final types = blockRegistry.turnIntoTypes;
      return SlashMenu(
        types: types,
        active: types.indexWhere((t) => t.kind == widget.kind),
        onPick: widget.onTurnInto,
        onHover: (_) {},
        title: 'Turn into',
        grouped: false,
      );
    }
    if (_sub == 2) return ColorMenu(current: widget.color, onPick: widget.onColor);
    final type = blockRegistry.of(widget.kind);
    final canTurn = blockRegistry.turnIntoTypes.any((t) => t.kind == widget.kind) || widget.count > 1;
    return MenuList(header: widget.count > 1 ? '${widget.count} blocks' : type.label, children: [
      if (canTurn)
        MenuItem(
            key: const ValueKey('menu-turn-into'),
            icon: Ico.turnInto,
            label: 'Turn into',
            onTap: () => setState(() => _sub = 1),
            trailing: Icon20(Ico.chevronRight, size: 16, color: c.iconTertiary)),
      if (type.colorable || widget.count > 1)
        MenuItem(
            key: const ValueKey('menu-color'),
            leading: IconX(Ix.color, size: 20, color: c.iconPrimary),
            label: 'Color',
            onTap: () => setState(() => _sub = 2),
            trailing: Icon20(Ico.chevronRight, size: 16, color: c.iconTertiary)),
      if (widget.kind == Kinds.todo && widget.count == 1)
        MenuItem(key: const ValueKey('menu-set-date'), icon: Ico.clock, label: 'Set date',
            onTap: () => widget.onAction(BlockAction.setDate)),
      if (canTurn || type.colorable) const MenuSeparator(),
      if (widget.onAskAi != null)
        MenuItem(key: const ValueKey('menu-ask-ai'), leading: const AiMark(size: 18),
            label: 'Ask AI', onTap: widget.onAskAi!),
      if (widget.onAskAgent != null)
        MenuItem(key: const ValueKey('menu-ask-agent'), icon: Ico.comment,
            label: 'Ask agent', onTap: widget.onAskAgent),
      MenuItem(
          key: const ValueKey('menu-copy'),
          icon: Ico.link,
          label: 'Copy',
          onTap: () => widget.onAction(BlockAction.copy),
          trailing: Text('Ctrl+C', style: ts(context.type.uiSmall, c.textTertiary, weight: 400))),
      MenuItem(
          key: const ValueKey('menu-duplicate'),
          icon: Ico.duplicate,
          label: 'Duplicate',
          onTap: () => widget.onAction(BlockAction.duplicate),
          trailing: Text('Ctrl+D', style: ts(context.type.uiSmall, c.textTertiary, weight: 400))),
      MenuItem(
          key: const ValueKey('menu-up'),
          icon: Ico.arrowUp,
          label: 'Move up',
          onTap: () => widget.onAction(BlockAction.moveUp)),
      MenuItem(
          key: const ValueKey('menu-down'),
          icon: Ico.arrowDown,
          label: 'Move down',
          onTap: () => widget.onAction(BlockAction.moveDown)),
      const MenuSeparator(),
      MenuItem(
          key: const ValueKey('menu-delete'),
          icon: Ico.trash,
          label: 'Delete',
          onTap: () => widget.onAction(BlockAction.delete),
          trailing: Text('Del', style: ts(context.type.uiSmall, c.textTertiary, weight: 400))),
    ]);
  }
}

// ---------------------------------------------------------------------------
// Block sheet (mobile, from a long press)

class SheetResult {
  const SheetResult(this.action, [this.kind]);
  final BlockAction action;

  /// The kind for turnInto, the colour for color.
  final String? kind;
}

class BlockSheet extends StatefulWidget {
  const BlockSheet({super.key, required this.kind, this.count = 1, this.color,
      this.onAskAgent, this.onAskAi});
  final String kind;
  final int count;
  final String? color;
  final VoidCallback? onAskAgent;

  /// Ask AI about the block(s); the answer goes after them.
  final VoidCallback? onAskAi;

  @override
  State<BlockSheet> createState() => _BlockSheetState();
}

class _BlockSheetState extends State<BlockSheet> {
  int _sub = 0;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    void pop(SheetResult r) => Navigator.of(context).pop(r);
    if (_sub == 2) {
      return ColorMenu(sheet: true, current: widget.color, onPick: (v) => pop(SheetResult(BlockAction.color, v)));
    }
    final type = blockRegistry.of(widget.kind);
    final children = _sub == 1
        ? [
            const SheetHeader('Turn into'),
            for (final t in blockRegistry.turnIntoTypes)
              SheetItem(
                key: ValueKey('sheet-kind-${t.kind}'),
                leading: KindIcon(t, size: 22, color: c.iconPrimary),
                label: t.label,
                selected: t.kind == widget.kind,
                onTap: () => pop(SheetResult(BlockAction.turnInto, t.kind)),
              ),
          ]
        : [
            if (widget.count > 1) SheetHeader('${widget.count} blocks'),
            if (widget.onAskAi != null)
              SheetItem(key: const ValueKey('sheet-ask-ai'), leading: const AiMark(size: 20),
                  label: 'Ask AI', onTap: () {
                    Navigator.of(context).pop();
                    widget.onAskAi!();
                  }),
            if (widget.onAskAgent != null)
              SheetItem(key: const ValueKey('sheet-ask-agent'), icon: Ico.comment,
                  label: 'Ask agent', onTap: () {
                    Navigator.of(context).pop();
                    widget.onAskAgent!();
                  }),
            SheetItem(
                key: const ValueKey('sheet-turn-into'),
                icon: Ico.turnInto,
                label: 'Turn into',
                onTap: () => setState(() => _sub = 1)),
            if (type.colorable || widget.count > 1)
              SheetItem(
                  key: const ValueKey('sheet-color'),
                  leading: IconX(Ix.color, size: 22, color: c.iconPrimary),
                  label: 'Color',
                  onTap: () => setState(() => _sub = 2)),
            if (widget.kind == Kinds.todo && widget.count == 1)
              SheetItem(key: const ValueKey('sheet-set-date'), icon: Ico.clock, label: 'Set date',
                  onTap: () => pop(const SheetResult(BlockAction.setDate))),
            if (widget.count == 1)
              SheetItem(
                  key: const ValueKey('sheet-select'),
                  leading: IconX(Ix.select, size: 22, color: c.iconPrimary),
                  label: 'Select',
                  onTap: () => pop(const SheetResult(BlockAction.select))),
            SheetItem(
                key: const ValueKey('sheet-up'),
                icon: Ico.arrowUp,
                label: 'Move up',
                onTap: () => pop(const SheetResult(BlockAction.moveUp))),
            SheetItem(
                key: const ValueKey('sheet-down'),
                icon: Ico.arrowDown,
                label: 'Move down',
                onTap: () => pop(const SheetResult(BlockAction.moveDown))),
            SheetItem(
                key: const ValueKey('sheet-duplicate'),
                icon: Ico.duplicate,
                label: 'Duplicate',
                onTap: () => pop(const SheetResult(BlockAction.duplicate))),
            SheetItem(
                key: const ValueKey('sheet-copy'),
                icon: Ico.link,
                label: 'Copy',
                onTap: () => pop(const SheetResult(BlockAction.copy))),
            Padding(padding: const EdgeInsets.symmetric(vertical: 4), child: HLine(color: c.borderSubtle)),
            SheetItem(
                key: const ValueKey('sheet-delete'),
                icon: Ico.trash,
                label: 'Delete',
                danger: true,
                onTap: () => pop(const SheetResult(BlockAction.delete))),
          ];
    return SheetSurface(children: children);
  }
}

// ---------------------------------------------------------------------------
// Link box

class LinkBox extends StatefulWidget {
  const LinkBox({super.key, required this.initial, required this.onDone});
  final String initial;
  final ValueChanged<String?> onDone;

  @override
  State<LinkBox> createState() => _LinkBoxState();
}

class _LinkBoxState extends State<LinkBox> {
  late final _ctl = TextEditingController(text: widget.initial);
  final _focus = FocusNode();

  @override
  void dispose() {
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return MenuSurface(
      width: 320,
      padding: const EdgeInsets.all(8),
      child: Container(
        height: 32,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(
          color: c.bgHover,
          borderRadius: BorderRadius.circular(Radii.md),
          border: Border.all(color: c.border),
        ),
        alignment: Alignment.centerLeft,
        child: PlainField(
          key: const ValueKey('link-field'),
          controller: _ctl,
          focusNode: _focus,
          autofocus: true,
          style: ts(ty.ui, c.textPrimary, weight: 400),
          placeholder: 'Paste link',
          onSubmitted: (v) => widget.onDone(v.trim()),
        ),
      ),
    );
  }
}
