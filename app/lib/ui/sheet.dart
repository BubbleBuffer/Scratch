// Bottom sheets (mobile): a route that slides up over a dimmed barrier and a
// surface with a grab handle that closes when dragged down.

import 'package:flutter/widgets.dart';

import 'hover.dart';
import 'icons.dart';
import 'tokens.dart';

class SheetRoute<T> extends PopupRoute<T> {
  SheetRoute({
    required this.builder,
    required this.barrier,
    this.duration = Motion.sheet,
  });
  final WidgetBuilder builder;
  final Color barrier;
  final Duration duration;

  @override
  Color? get barrierColor => barrier;
  @override
  bool get barrierDismissible => true;
  @override
  String? get barrierLabel => 'Close';
  @override
  Duration get transitionDuration => duration;

  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) => Align(alignment: Alignment.bottomCenter, child: builder(context));

  @override
  Widget buildTransitions(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) {
    if (MediaQuery.disableAnimationsOf(context)) return child;
    final t = CurvedAnimation(
      parent: animation,
      curve: Motion.easing,
      reverseCurve: Motion.easing.flipped,
    );
    return SlideTransition(
      position: Tween(begin: const Offset(0, 1), end: Offset.zero).animate(t),
      child: child,
    );
  }
}

/// Shows [builder] in a bottom sheet; completes with what it pops.
Future<T?> showSheet<T>(BuildContext context, WidgetBuilder builder) =>
    Navigator.of(context).push(
      SheetRoute<T>(
        builder: builder,
        barrier: context.colors.overlay,
        duration: Motion.of(context).sheet,
      ),
    );

/// The sheet's surface: rounded top, grab handle, drag down to close.
class SheetSurface extends StatefulWidget {
  const SheetSurface({super.key, required this.children});
  final List<Widget> children;

  @override
  State<SheetSurface> createState() => _SheetSurfaceState();
}

class _SheetSurfaceState extends State<SheetSurface> {
  double _drag = 0;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final mq = MediaQuery.of(context);
    return Transform.translate(
      offset: Offset(0, _drag),
      child: GestureDetector(
        onVerticalDragUpdate: (d) =>
            setState(() => _drag = (_drag + d.delta.dy).clamp(0, 600)),
        onVerticalDragEnd: (d) {
          if (_drag > 120 || d.velocity.pixelsPerSecond.dy > 700) {
            Navigator.of(context).pop();
          } else {
            setState(() => _drag = 0);
          }
        },
        child: Container(
          width: double.infinity,
          constraints: BoxConstraints(maxHeight: mq.size.height * 0.85),
          decoration: BoxDecoration(
            color: c.bgElevated,
            borderRadius: Radii.panel,
            boxShadow: c.seamHover,
          ),
          padding: EdgeInsets.only(bottom: mq.padding.bottom + 8),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const SizedBox(height: 8),
              Center(
                child: Container(
                  width: 36,
                  height: 4,
                  decoration: BoxDecoration(
                    color: c.borderStrong,
                    borderRadius: BorderRadius.circular(2),
                  ),
                ),
              ),
              const SizedBox(height: 12),
              Flexible(
                child: SingleChildScrollView(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: widget.children,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The grey label above a group of sheet rows.
class SheetHeader extends StatelessWidget {
  const SheetHeader(this.text, {super.key});
  final String text;
  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(Layout.gutter, 4, Layout.gutter, 8),
    child: Text(
      text,
      style: ts(context.type.uiSmall, context.colors.textTertiary),
    ),
  );
}

/// One 48 px sheet row: icon, label, a check when [selected].
class SheetItem extends StatelessWidget {
  const SheetItem({
    super.key,
    required this.label,
    required this.onTap,
    this.icon,
    this.leading,
    this.selected = false,
    this.danger = false,
  });
  final String label;
  final VoidCallback onTap;
  final Ico? icon;
  final Widget? leading;
  final bool selected;
  final bool danger;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final color = danger ? c.inlineCodeText : null;
    return Hover(
      onTap: onTap,
      borderRadius: Radii.ctl,
      selected: selected,
      padding: const EdgeInsets.symmetric(horizontal: Layout.gutter),
      child: SizedBox(
        height: 48,
        child: Row(
          children: [
            leading ??
                (icon == null
                    ? const SizedBox(width: 22)
                    : Icon20(icon!, size: 22, color: color ?? c.iconPrimary)),
            const SizedBox(width: 14),
            Expanded(
              child: Text(
                label,
                style: ts(context.type.body, color ?? c.textPrimary),
              ),
            ),
            if (selected) Icon20(Ico.check, size: 18, color: c.accentText),
          ],
        ),
      ),
    );
  }
}
