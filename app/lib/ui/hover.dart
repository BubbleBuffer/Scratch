// Shared interactive surface: consistent lift, press, focus and reduced motion.
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'tokens.dart';

class Hover extends StatefulWidget {
  const Hover({
    super.key,
    required this.child,
    this.onTap,
    this.onLongPress,
    this.onSecondaryTap,
    this.radius = Radii.md,
    this.borderRadius,
    this.selected = false,
    this.selectedColor,
    this.padding = EdgeInsets.zero,
    this.cursor = SystemMouseCursors.click,
    this.hoverColor,
    this.onHoverChanged,
    this.lift = 1,
    this.shadow,
    this.hoverShadow,
    this.background,
    this.hoverBackground,
    this.pressedBackground,
    this.border,
  });
  final Widget child;
  final VoidCallback? onTap, onLongPress, onSecondaryTap;
  final double radius, lift;
  final BorderRadius? borderRadius;
  final bool selected;
  final Color? selectedColor,
      hoverColor,
      background,
      hoverBackground,
      pressedBackground;
  final EdgeInsetsGeometry padding;
  final MouseCursor cursor;
  final ValueChanged<bool>? onHoverChanged;
  final List<BoxShadow>? shadow, hoverShadow;
  final BoxBorder? border;
  @override
  State<Hover> createState() => _HoverState();
}

class _HoverState extends State<Hover> {
  bool _hover = false, _down = false, _focus = false;
  void _setHover(bool v) {
    setState(() => _hover = v);
    widget.onHoverChanged?.call(v);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final motion = Motion.of(context);
    final interactive =
        widget.onTap != null ||
        widget.onLongPress != null ||
        widget.onSecondaryTap != null;
    final radius =
        widget.borderRadius ??
        (widget.radius == Radii.md
            ? Radii.ctl
            : BorderRadius.circular(widget.radius));
    final bg = _down
        ? (widget.pressedBackground ?? c.bgPressed)
        : widget.selected
        ? (widget.selectedColor ?? c.bgHover)
        : _hover && interactive
        ? (widget.hoverBackground ?? widget.hoverColor ?? c.bgHover)
        : widget.background;
    return FocusableActionDetector(
      enabled: widget.onTap != null,
      onShowFocusHighlight: (value) => setState(() => _focus = value),
      shortcuts: const {
        SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
        SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
      },
      actions: {
        ActivateIntent: CallbackAction<ActivateIntent>(
          onInvoke: (_) {
            widget.onTap?.call();
            return null;
          },
        ),
      },
      child: MouseRegion(
        cursor: interactive ? widget.cursor : MouseCursor.defer,
        onEnter: (_) => _setHover(true),
        onExit: (_) => _setHover(false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onTap,
          onLongPress: widget.onLongPress,
          onSecondaryTap: widget.onSecondaryTap,
          onTapDown: widget.onTap == null
              ? null
              : (_) => setState(() => _down = true),
          onTapUp: widget.onTap == null
              ? null
              : (_) => setState(() => _down = false),
          onTapCancel: widget.onTap == null
              ? null
              : () => setState(() => _down = false),
          child: AnimatedContainer(
            duration: motion.fast,
            curve: Motion.ease,
            transform: Matrix4.translationValues(
              0,
              motion.reduced
                  ? 0
                  : _down
                  ? 1
                  : _hover && interactive
                  ? -widget.lift
                  : 0,
              0,
            ),
            decoration: BoxDecoration(
              color: bg,
              borderRadius: radius,
              border: widget.border,
              boxShadow: _hover && interactive
                  ? (widget.hoverShadow ?? widget.shadow)
                  : widget.shadow,
            ),
            child: CustomPaint(
              foregroundPainter: _focus ? _FocusRing(c.focus, radius) : null,
              child: Padding(padding: widget.padding, child: widget.child),
            ),
          ),
        ),
      ),
    );
  }
}

class _FocusRing extends CustomPainter {
  const _FocusRing(this.color, this.radius);
  final Color color;
  final BorderRadius radius;
  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRRect(
      radius.toRRect((Offset.zero & size).inflate(3)),
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2,
    );
  }

  @override
  bool shouldRepaint(_FocusRing old) =>
      old.color != color || old.radius != radius;
}
