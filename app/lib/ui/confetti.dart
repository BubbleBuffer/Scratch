import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import 'tokens.dart';

/// Changing [trigger] celebrates one user completion. Initial state is quiet.
class ConfettiBurst extends StatefulWidget {
  const ConfettiBurst({
    super.key,
    required this.trigger,
    required this.patch,
    required this.child,
    this.seed,
  });
  final int trigger;
  final SpacePatch patch;
  final Widget child;
  final int? seed;
  @override
  State<ConfettiBurst> createState() => ConfettiBurstState();
}

class ConfettiBurstState extends State<ConfettiBurst>
    with SingleTickerProviderStateMixin {
  late final AnimationController animation = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 900),
  );
  late List<ConfettiPiece> pieces;
  int bursts = 0;
  @override
  void initState() {
    super.initState();
    _makePieces();
  }

  void _makePieces() {
    final random = math.Random(
      widget.seed ?? (kDebugMode ? 7 : DateTime.now().microsecondsSinceEpoch),
    );
    pieces = List.generate(
      14,
      (n) => ConfettiPiece(
        (n - 6.5) * 12 + random.nextDouble() * 8 - 4,
        24 + random.nextDouble() * 24,
        70 + random.nextDouble() * 30,
        6 + random.nextDouble() * 3,
        6 + random.nextDouble() * 3,
        random.nextDouble() * math.pi * 2,
      ),
    );
  }

  @override
  void didUpdateWidget(ConfettiBurst oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.trigger > oldWidget.trigger && !Motion.of(context).reduced) {
      _makePieces();
      bursts++;
      animation.forward(from: 0);
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (Motion.of(context).reduced) animation.stop();
  }

  @override
  void dispose() {
    animation.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Stack(
    clipBehavior: Clip.none,
    children: [
      widget.child,
      Positioned.fill(
        child: IgnorePointer(
          child: AnimatedBuilder(
            animation: animation,
            builder: (context, _) =>
                !animation.isAnimating || Motion.of(context).reduced
                ? const SizedBox.shrink()
                : CustomPaint(
                    key: const ValueKey('confetti-pieces'),
                    painter: ConfettiPainter(
                      fraction: animation.value,
                      patch: widget.patch,
                      pieces: pieces,
                    ),
                  ),
          ),
        ),
      ),
    ],
  );
}

class ConfettiPiece {
  const ConfettiPiece(
    this.x,
    this.lift,
    this.fall,
    this.width,
    this.height,
    this.rotation,
  );
  final double x, lift, fall, width, height, rotation;
}

class ConfettiPainter extends CustomPainter {
  const ConfettiPainter({
    required this.fraction,
    required this.patch,
    required this.pieces,
  });
  final double fraction;
  final SpacePatch patch;
  final List<ConfettiPiece> pieces;
  @override
  void paint(Canvas canvas, Size size) {
    final t = Curves.easeOut.transform(fraction.clamp(0.0, 1.0));
    final first = t < .27;
    final phase = first ? t / .27 : (t - .27) / .73;
    for (var n = 0; n < pieces.length; n++) {
      final p = pieces[n];
      final x = first ? p.x * .55 * phase : p.x * (.55 + .45 * phase);
      final y = first ? -p.lift * phase : -p.lift + (p.fall + p.lift) * phase;
      canvas.save();
      canvas.translate(size.width * .7 + x, size.height * .5 + y);
      canvas.rotate(p.rotation * t);
      canvas.drawRRect(
        RRect.fromRectAndCorners(
          Rect.fromLTWH(0, 0, p.width, p.height),
          topLeft: const Radius.circular(2),
          topRight: const Radius.circular(4),
          bottomLeft: const Radius.circular(3),
          bottomRight: const Radius.circular(1),
        ),
        Paint()
          ..color = (n.isEven ? patch.fill : patch.deep).withValues(
            alpha: first ? 1 : 1 - phase,
          ),
      );
      canvas.restore();
    }
  }

  @override
  bool shouldRepaint(ConfettiPainter old) =>
      old.fraction != fraction || old.patch != patch || old.pieces != pieces;
}
