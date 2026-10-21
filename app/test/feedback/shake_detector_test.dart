import 'dart:math' as math;

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/feedback/shake_detector.dart';

const _gravity = 9.80665;

final _start = DateTime(2026, 10, 16, 9);

void main() {
  var now = 0; // ms of the sample fed last, read by onShake
  var suppressed = false;
  final fired = <int>[];
  late ShakeDetector detector;

  setUp(() {
    now = 0;
    suppressed = false;
    fired.clear();
    detector = ShakeDetector(
      onShake: () => fired.add(now),
      isSuppressed: () => suppressed,
    );
  });

  /// Feeds a sample of [g] G-force at [ms], spread across the three axes.
  void feed(double g, int ms) {
    now = ms;
    final each = g * _gravity / math.sqrt(3);
    detector.onSample(each, each, each, _start.add(Duration(milliseconds: ms)));
  }

  test('motion below the threshold never fires', () {
    for (var ms = 0; ms <= 10000; ms += 20) {
      feed((ms ~/ 100).isEven ? 1.0 : 2.6, ms);
    }
    expect(fired, isEmpty);
  });

  test('a single peak does not fire', () {
    feed(3.5, 0);
    feed(1.0, 100);
    expect(fired, isEmpty);
  });

  test('a real shake fires exactly once', () {
    for (final ms in [0, 150, 300, 450, 600, 750]) {
      feed(3.5, ms);
    }
    expect(fired, [150]);
  });

  test('a second shake inside the cooldown is ignored, one after it fires', () {
    feed(3.5, 0);
    feed(3.5, 150);
    expect(fired, [150]);

    feed(3.5, 1000);
    feed(3.5, 1150);
    expect(fired, [150]);

    feed(3.5, 2500);
    feed(3.5, 2650);
    expect(fired, [150, 2650]);
  });

  test('one long jolt does not double trigger', () {
    for (var ms = 0; ms <= 200; ms += 10) {
      feed(3.5, ms);
    }
    expect(fired, isEmpty);

    // The jolt is one peak, so one more peak inside the window makes one shake.
    feed(3.5, 400);
    expect(fired, [400]);
  });

  test('a shake while suppressed does not fire', () {
    suppressed = true;
    for (final ms in [0, 150, 300]) {
      feed(3.5, ms);
    }
    expect(fired, isEmpty);
  });

  test('resuming does not fire from peaks seen before suppression', () {
    feed(3.5, 0);
    suppressed = true;
    feed(3.5, 100);
    feed(3.5, 150);
    suppressed = false;

    // The peak at 0 is still inside the window, so it must have been dropped.
    feed(3.5, 250);
    expect(fired, isEmpty);

    feed(3.5, 400);
    expect(fired, [400]);
  });

  test('peaks spread wider than the window do not fire', () {
    for (final ms in [0, 600, 1200, 1800]) {
      feed(3.5, ms);
    }
    expect(fired, isEmpty);
  });
}
