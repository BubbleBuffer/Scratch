// Mobile Home's side panels. Home stays mounted underneath. Swiping right
// reveals the page tree from the left and swiping left reveals the agent chat
// full width from the right. Both follow the finger and settle on release:
// past half the panel, or a fling faster than 700 px/s, opens it; otherwise it
// returns closed. Scrim taps and Android back close the tree first.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../ui/mobile_navigation_gesture.dart';
import '../ui/ui.dart';
import 'page_stack.dart';

/// The page tree panel: 85% of the screen, at most 360 px.
double mobileTreePanelWidth(double screen) => math.min(screen * .85, 360);

/// Releases faster than this (px/s) decide the side on their own.
const _flingSpeed = 700.0;

class MobileHomePanels extends StatefulWidget {
  const MobileHomePanels({
    super.key,
    required this.home,
    required this.tree,
    required this.agent,
    this.hasAgent = true,
  });

  /// Home, with its dock. Never removed, so its scroll position survives.
  final Widget home;

  /// The page tree (with Trash and Settings), at the left edge.
  final Widget tree;

  /// The agent chat, full width, at the right edge.
  final Widget agent;
  final bool hasAgent;

  @override
  State<MobileHomePanels> createState() => MobileHomePanelsState();
}

class MobileHomePanelsState extends State<MobileHomePanels>
    with SingleTickerProviderStateMixin {
  late final AnimationController _ctl = AnimationController(vsync: this)
    ..addListener(_tick);
  PageStackNavigator? _stack;

  /// Offset in px. Above 0, the tree shows that far from the left edge; below
  /// 0, the agent covers that far from the right edge.
  double _x = 0;

  /// The side the panels have settled on: 1 tree, -1 agent, 0 Home alone.
  int _side = 0;
  double _from = 0, _to = 0;

  /// The side a drag started from.
  int _dragFrom = 0;

  /// The agent is built on first use, so its chat loads only when shown.
  bool _agentSeen = false;

  /// Opens the page tree (the dock's tree button).
  void openTree() => _settle(1);

  /// Closes whichever panel is open.
  void close() => _settle(0);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final stack = PageStackScope.maybeOf(context);
    if (identical(stack, _stack)) return;
    _stack?.removeListener(_stackChanged);
    _stack = stack;
    stack?.addListener(_stackChanged);
  }

  @override
  void dispose() {
    _stack?.removeListener(_stackChanged);
    _ctl.dispose();
    super.dispose();
  }

  double _positionOf(int side) => switch (side) {
    1 => mobileTreePanelWidth(MediaQuery.sizeOf(context).width),
    -1 => -MediaQuery.sizeOf(context).width,
    _ => 0,
  };

  void _settle(int side) {
    if (side == -1 && !widget.hasAgent) side = 0;
    _ctl.stop();
    FocusManager.instance.primaryFocus?.unfocus();
    if (side == -1) _agentSeen = true;
    final motion = Motion.of(context);
    final target = _positionOf(side);
    setState(() => _side = side);
    if (motion.reduced) {
      setState(() => _x = target);
      return;
    }
    _from = _x;
    _to = target;
    _ctl.duration = motion.sheet;
    _ctl.forward(from: 0);
  }

  void _tick() {
    final t = Motion.easing.transform(_ctl.value);
    setState(() => _x = _from + (_to - _from) * t);
  }

  void _dragStart() {
    _ctl.stop();
    _dragFrom = _side;
  }

  void _dragUpdate(DragUpdateDetails d) {
    final w = MediaQuery.sizeOf(context).width;
    final low = _dragFrom == 1 || !widget.hasAgent ? 0.0 : -w;
    final high = _dragFrom == -1 ? 0.0 : mobileTreePanelWidth(w);
    setState(() => _x = (_x + d.delta.dx).clamp(low, high).toDouble());
    if (_x < 0) _agentSeen = true;
  }

  void _dragEnd(DragEndDetails d) {
    final v = d.velocity.pixelsPerSecond.dx;
    final w = MediaQuery.sizeOf(context).width;
    final tree = mobileTreePanelWidth(w);
    final int target;
    switch (_dragFrom) {
      case 1:
        target = v < -_flingSpeed || _x < tree / 2 ? 0 : 1;
      case -1:
        target = v > _flingSpeed || _x > -w / 2 ? 0 : -1;
      default:
        if (v > _flingSpeed) {
          target = 1;
        } else if (v < -_flingSpeed) {
          target = -1;
        } else if (_x > tree / 2) {
          target = 1;
        } else if (_x < -w / 2) {
          target = -1;
        } else {
          target = 0;
        }
    }
    _settle(target);
  }

  /// A page opened (from the tree, search, Today or the agent): the panels
  /// close at once, so returning from the page lands on Home.
  void _stackChanged() {
    final stack = _stack;
    final top = stack?.currentRoute;
    if (stack == null || top == null || stack.idOf(top) == null) return;
    if (_side == 0 && _x == 0) return;
    _ctl.stop();
    setState(() {
      _side = 0;
      _x = 0;
    });
  }

  @override
  Widget build(BuildContext context) => _panels(context);

  Widget _panels(BuildContext context) {
    final mq = MediaQuery.of(context);
    final c = context.colors;
    final w = mq.size.width;
    final tree = mobileTreePanelWidth(w);
    final scrim = (_x.abs() / (_x > 0 ? tree : w)).clamp(0.0, 1.0).toDouble();
    return PopScope(
      // Android back closes an open panel before it reaches Home.
      canPop: _side == 0 && _x == 0,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) _settle(0);
      },
      child: MobileNavigationGesture(
        onStart: _dragStart,
        onUpdate: _dragUpdate,
        onEnd: _dragEnd,
        onCancel: () => _settle(_dragFrom),
        child: Stack(
          children: [
            Positioned.fill(
              child: IgnorePointer(
                ignoring: _x != 0,
                child: ExcludeFocus(
                  excluding: _x != 0,
                  child: ExcludeSemantics(
                    excluding: _x != 0,
                    child: widget.home,
                  ),
                ),
              ),
            ),
            if (_x != 0)
              Positioned.fill(
                child: GestureDetector(
                  key: const ValueKey('mobile-panel-scrim'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => _settle(0),
                  child: ColoredBox(
                    color: (c.dark ? const Color(0xFF000000) : c.textPrimary)
                        .withValues(alpha: (c.dark ? .4 : .2) * scrim),
                  ),
                ),
              ),
            Positioned(
              left: _x - tree,
              top: 0,
              bottom: 0,
              width: tree,
              child: Offstage(
                offstage: _x <= 0,
                child: ExcludeFocus(
                  excluding: _x <= 0,
                  child: Container(
                    key: const ValueKey('mobile-tree-panel'),
                    decoration: BoxDecoration(
                      color: c.desk,
                      boxShadow: c.stack,
                    ),
                    child: Padding(
                      padding: EdgeInsets.only(top: mq.padding.top),
                      child: widget.tree,
                    ),
                  ),
                ),
              ),
            ),
            if (_agentSeen)
              Positioned(
                left: _x + w,
                top: 0,
                bottom: 0,
                width: w,
                child: Offstage(
                  offstage: _x >= 0,
                  child: ExcludeFocus(
                    excluding: _x >= 0,
                    child: Container(
                      key: const ValueKey('mobile-agent-panel'),
                      decoration: BoxDecoration(
                        color: c.desk,
                        boxShadow: c.stack,
                      ),
                      child: Padding(
                        padding: EdgeInsets.only(
                          top: mq.padding.top,
                          bottom: mq.viewInsets.bottom,
                        ),
                        child: widget.agent,
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
