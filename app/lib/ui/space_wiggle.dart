import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import 'tokens.dart';

/// One small wiggle when entering a space tile; never loops while hovered.
class SpaceWiggle extends StatefulWidget {
  const SpaceWiggle({super.key, required this.child});
  final Widget child;
  @override
  State<SpaceWiggle> createState() => _SpaceWiggleState();
}

class _SpaceWiggleState extends State<SpaceWiggle>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(vsync: this);
  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final duration = Motion.of(context).wiggle;
    if (duration == Duration.zero) {
      _controller.stop();
      return widget.child;
    }
    return MouseRegion(
      onEnter: (_) {
        _controller.duration = duration;
        _controller.forward(from: 0);
      },
      child: AnimatedBuilder(
        animation: _controller,
        child: widget.child,
        builder: (_, child) => Transform.rotate(
          angle: math.sin(_controller.value * math.pi * 2) * 3 * math.pi / 180,
          child: child,
        ),
      ),
    );
  }
}
