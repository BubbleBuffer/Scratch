// Page routes and scrolling behaviour.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';

import 'tokens.dart';

/// Mobile pages slide in from the right. Desktop sheet routes stay transparent
/// and animate their sheet inside the frame so the rail remains fixed.
Route<T> pageRoute<T>(
  WidgetBuilder builder, {
  required bool mobile,
  required MotionDurations motion,
  RouteSettings? settings,
  bool desktopSheet = false,
}) => PageRouteBuilder<T>(
  settings: settings,
  opaque: !desktopSheet,
  maintainState: true,
  transitionDuration: mobile || desktopSheet ? motion.sheet : Duration.zero,
  reverseTransitionDuration: mobile
      ? motion.standard
      : desktopSheet
      ? motion.sheet
      : Duration.zero,
  pageBuilder: (c, a, b) => builder(c),
  transitionsBuilder: (c, a, b, child) {
    if ((!mobile && !desktopSheet) || MediaQuery.disableAnimationsOf(c)) {
      return child;
    }
    if (!mobile) return child; // The desktop frame moves only the sheet, leaving the rail fixed.
    final t = CurvedAnimation(parent: a, curve: Motion.easing);
    return SlideTransition(
      position: Tween(begin: const Offset(1, 0), end: Offset.zero).animate(t),
      child: mobile
          ? DecoratedBox(
              decoration: BoxDecoration(boxShadow: c.colors.stack),
              child: child,
            )
          : child,
    );
  },
);

/// No glow or stretch overscroll; a thin scrollbar on desktop.
class AppScrollBehavior extends ScrollBehavior {
  const AppScrollBehavior();

  @override
  Set<PointerDeviceKind> get dragDevices => {
    PointerDeviceKind.touch,
    PointerDeviceKind.stylus,
    PointerDeviceKind.invertedStylus,
    PointerDeviceKind.trackpad,
  };

  @override
  Widget buildOverscrollIndicator(
    BuildContext context,
    Widget child,
    ScrollableDetails details,
  ) => child;

  @override
  Widget buildScrollbar(
    BuildContext context,
    Widget child,
    ScrollableDetails details,
  ) {
    if (context.mobile ||
        details.direction == AxisDirection.left ||
        details.direction == AxisDirection.right) {
      return child;
    }
    final c = context.colors;
    return RawScrollbar(
      controller: details.controller,
      thumbColor: c.dark ? const Color(0x33FFFFFF) : const Color(0x26000000),
      thickness: 6,
      radius: const Radius.circular(3),
      crossAxisMargin: 2,
      child: child,
    );
  }

  @override
  ScrollPhysics getScrollPhysics(BuildContext context) =>
      const ClampingScrollPhysics();
}
