// Small parts every database screen shares: option pills, the checkbox,
// icon buttons with database icons, menu rows, the boxed menu field, a
// toggle, and [showPanel] (a popover on desktop, a bottom sheet on mobile).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../ui.dart';
import 'db_icons.dart';
import 'db_theme.dart';
import 'config.dart';

/// A select, multi-select or status option as a pill.
class Pill extends StatelessWidget {
  const Pill(this.label, {super.key, this.color, this.status = false, this.small = false,
    this.group});
  final String label;
  final String? color;
  final StatusGroup? group;

  /// Status pills are fully rounded with a dot before the label.
  final bool status;

  /// Board and gallery cards use a smaller pill (12px).
  final bool small;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final st = status && group != null
        ? TagStyle(switch (group!) {
            StatusGroup.todo => c.bgSecondary,
            StatusGroup.inProgress => c.infoBg,
            StatusGroup.complete => c.okBg,
          }, c.textPrimary)
        : c.tag(color);
    const size = 12.0;
    final h = small ? 18.0 : 22.0;
    return Container(
      height: h,
      padding: EdgeInsets.symmetric(horizontal: small ? 6 : (m ? 8 : 6)),
      decoration: BoxDecoration(color: st.bg, borderRadius: Radii.pill),
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        if (status) ...[
          Container(
              width: small ? 6 : 8,
              height: small ? 6 : 8,
              decoration: BoxDecoration(color: c.tone(color), shape: BoxShape.circle)),
          SizedBox(width: small ? 4 : 5),
        ],
        Flexible(
          child: Text(label.isEmpty ? ' ' : label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: tsRaw(size, h, 600, st.text)),
        ),
      ]),
    );
  }
}

/// The 16 px checkbox of checkbox properties.
class DbCheckbox extends StatelessWidget {
  const DbCheckbox({super.key, required this.checked, this.size = 16});
  final bool checked;
  final double size;

  @override
  Widget build(BuildContext context) {
    return StacksCheckbox(checked: checked, size: size,
    );
  }
}

/// A square button with a database icon.
class DbIconBtn extends StatelessWidget {
  const DbIconBtn(this.icon,
      {super.key, this.onTap, this.size = 28, this.glyph = 16, this.color, this.selected = false, this.label});
  final DbIco icon;
  final VoidCallback? onTap;
  final double size, glyph;
  final Color? color;
  final bool selected;
  final String? label;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      button: true,
      label: label ?? icon.name,
      child: Hover(
        onTap: onTap,
        radius: Radii.md,
        selected: selected,
        child: SizedBox(
            width: size,
            height: size,
            child: Center(child: DbIcon(icon, size: glyph, color: color ?? (selected ? c.accent : c.iconSecondary)))),
      ),
    );
  }
}

/// A menu row with a database icon.
MenuItem dbItem(BuildContext context, String label, DbIco? icon,
        {Key? key, VoidCallback? onTap, Widget? trailing, bool selected = false, bool danger = false, Ico? ico}) =>
    MenuItem(
      key: key,
      label: label,
      onTap: onTap,
      icon: ico,
      leading: icon == null
          ? null
          : DbIcon(icon, size: context.mobile ? 22 : 20, color: danger ? context.colors.inlineCodeText : context.colors.iconPrimary),
      trailing: trailing,
      selected: selected,
      danger: danger,
    );

/// The grey value and chevron at the end of a menu row ("Type  Text ›").
Widget menuValue(BuildContext context, String value, {bool chevron = true}) {
  final c = context.colors;
  return Row(mainAxisSize: MainAxisSize.min, children: [
    ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 140),
      child: Text(value,
          maxLines: 1, overflow: TextOverflow.ellipsis, style: ts(context.type.ui, c.textTertiary, weight: 400)),
    ),
    if (chevron) ...[const SizedBox(width: 2), Icon20(Ico.chevronRight, size: 16, color: c.iconTertiary)],
  ]);
}

/// A boxed field for menus ("Filter by…", rename, values).
class MenuField extends StatefulWidget {
  const MenuField({
    super.key,
    this.controller,
    this.initial = '',
    this.placeholder = '',
    this.onChanged,
    this.onSubmitted,
    this.autofocus = false,
    this.leading,
    this.numeric = false,
  });
  final TextEditingController? controller;
  final String initial;
  final String placeholder;
  final ValueChanged<String>? onChanged;
  final ValueChanged<String>? onSubmitted;
  final bool autofocus;
  final Widget? leading;
  final bool numeric;

  @override
  State<MenuField> createState() => _MenuFieldState();
}

class _MenuFieldState extends State<MenuField> {
  late final TextEditingController _ctl = widget.controller ?? TextEditingController(text: widget.initial);
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.addListener(() => setState(() {}));
  }

  @override
  void dispose() {
    if (widget.controller == null) _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    return FieldFrame(
      height: m ? 36 : 28,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(children: [
        if (widget.leading != null) ...[widget.leading!, const SizedBox(width: 6)],
        Expanded(
          child: PlainField(
            controller: _ctl,
            focusNode: _focus,
            style: ts(m ? context.type.body : context.type.ui, c.textPrimary, weight: 400),
            placeholder: widget.placeholder,
            onChanged: widget.onChanged,
            onSubmitted: widget.onSubmitted,
            autofocus: widget.autofocus,
            textInputAction: TextInputAction.done,
          ),
        ),
      ]),
    );
  }
}

/// An on/off switch (view settings).
class Toggle extends StatelessWidget {
  const Toggle({super.key, required this.on});
  final bool on;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return AnimatedContainer(
      duration: Motion.of(context).fast,
      width: 26,
      height: 14,
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: on ? c.accent : (c.dark ? const Color(0x33FFFFFF) : const Color(0x29000000)),
        borderRadius: BorderRadius.circular(7),
      ),
      alignment: on ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
          width: 10, height: 10, decoration: const BoxDecoration(color: Color(0xFFFFFFFF), shape: BoxShape.circle)),
    );
  }
}

/// Shows [content] as a popover under [anchor] (desktop) or in a bottom
/// sheet (mobile). Completes with what it pops.
Future<T?> showPanel<T>(BuildContext context, Rect anchor, WidgetBuilder content, {double width = 265}) {
  if (context.mobile) {
    return showSheet<T>(
      context,
      (ctx) => SheetSurface(children: [
        Padding(padding: const EdgeInsets.symmetric(horizontal: 10), child: content(ctx)),
      ]),
    );
  }
  return showPopover<T>(
    context,
    anchor,
    (ctx) => MenuSurface(
      width: width,
      padding: const EdgeInsets.all(6),
      child: content(ctx),
    ),
  );
}

/// Shows [content] over [anchor] (a cell editor): desktop only, at the
/// cell's top-left, at least the cell's width.
Future<T?> showOverCell<T>(BuildContext context, Rect anchor, WidgetBuilder content, {double? width}) {
  if (context.mobile) return showPanel<T>(context, anchor, content);
  return Navigator.of(context).push(_OverCellRoute<T>(anchor, width, content, Motion.of(context).fast));
}

class _OverCellRoute<T> extends PopupRoute<T> {
  _OverCellRoute(this.anchor, this.width, this.builder, this.duration);
  final Duration duration;
  final Rect anchor;
  final double? width;
  final WidgetBuilder builder;

  @override
  Color? get barrierColor => null;
  @override
  bool get barrierDismissible => true;
  @override
  String? get barrierLabel => 'Close';
  @override
  Duration get transitionDuration => duration;

  @override
  Widget buildPage(BuildContext context, Animation<double> animation, Animation<double> secondaryAnimation) {
    final w = width ?? (anchor.width < 240 ? 240.0 : anchor.width);
    return CustomSingleChildLayout(
      delegate: _OverCellLayout(anchor, w),
      child: MenuSurface(child: builder(context)),
    );
  }

  @override
  Widget buildTransitions(
          BuildContext context, Animation<double> animation, Animation<double> secondaryAnimation, Widget child) =>
      FadeTransition(opacity: animation, child: child);
}

/// At the cell's top-left, moved left and up as needed to stay on screen.
class _OverCellLayout extends SingleChildLayoutDelegate {
  _OverCellLayout(this.anchor, this.width);
  final Rect anchor;
  final double width;

  static const margin = 8.0;

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints c) =>
      BoxConstraints.tightFor(width: width).copyWith(maxHeight: c.maxHeight - 2 * margin);

  @override
  Offset getPositionForChild(Size size, Size child) {
    final left = anchor.left.clamp(margin, (size.width - child.width - margin).clamp(margin, double.infinity));
    final top = anchor.top.clamp(margin, (size.height - child.height - margin).clamp(margin, double.infinity));
    return Offset(left, top);
  }

  @override
  bool shouldRelayout(_OverCellLayout old) => old.anchor != anchor || old.width != width;
}

/// A small rounded button with text (toolbar "New", rule pills).
class ChipBtn extends StatelessWidget {
  const ChipBtn({
    super.key,
    required this.label,
    this.onTap,
    this.icon,
    this.chevron = false,
    this.active = false,
    this.radius = 14,
    this.height = 24,
  });
  final String label;
  final VoidCallback? onTap;
  final DbIco? icon;
  final bool chevron;

  /// Tinted blue (a rule that applies).
  final bool active;
  final double radius, height;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final fg = active ? c.accent : c.textSecondary;
    return Hover(
      onTap: onTap,
      radius: radius,
      selected: active,
      selectedColor: c.accent.withValues(alpha: c.dark ? 0.16 : 0.07),
      padding: const EdgeInsets.symmetric(horizontal: 8),
      child: Container(
        height: height,
        decoration: active ? null : null,
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          if (icon != null) ...[DbIcon(icon!, size: 14, color: fg), const SizedBox(width: 4)],
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 220),
            child: Text(label,
                maxLines: 1, overflow: TextOverflow.ellipsis, style: ts(context.type.uiSmall, fg, weight: 500)),
          ),
          if (chevron) ...[const SizedBox(width: 2), Icon20(Ico.chevronDown, size: 12, color: fg)],
        ]),
      ),
    );
  }
}

/// A back row at the top of a sub-menu: "‹ Title".
Widget menuBack(BuildContext context, String title, VoidCallback onBack) {
  final c = context.colors;
  return Padding(
    padding: const EdgeInsets.only(bottom: 4),
    child: Row(children: [
      IconBtn(Ico.chevronLeft, onTap: onBack, size: 24, glyph: 16, semanticLabel: 'Back'),
      const SizedBox(width: 4),
      Expanded(child: Text(title, style: ts(context.type.ui, c.textPrimary, weight: 600))),
    ]),
  );
}
