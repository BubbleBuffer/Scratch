// Turns accelerometer samples into a single shake event. Pure logic: the
// caller passes each sample's own time, so there is no clock to fake in tests
// and no sensor import here.

import 'dart:math' as math;

/// Standard gravity in m/s^2, the unit a G-force is counted in.
const double _gravity = 9.80665;

class ShakeDetector {
  ShakeDetector({
    required this.onShake,
    this.threshold = 2.7,
    this.peakCount = 2,
    this.window = const Duration(milliseconds: 500),
    this.cooldown = const Duration(seconds: 2),
    this.minPeakSpacing = const Duration(milliseconds: 80),
    this.isSuppressed,
  }) : assert(threshold > 0),
       assert(peakCount >= 1);

  /// Called once for each shake.
  final void Function() onShake;

  /// G-force a sample must exceed to count as a peak.
  final double threshold;

  /// Peaks needed inside [window] to make a shake.
  final int peakCount;

  /// Longest span in which [peakCount] peaks still make one shake.
  final Duration window;

  /// Time after a shake during which every sample is ignored.
  final Duration cooldown;

  /// Above-threshold samples this close together are one jolt, not many peaks.
  final Duration minPeakSpacing;

  /// While this returns true, samples are ignored and pending peaks are dropped.
  final bool Function()? isSuppressed;

  final _peaks = <DateTime>[];
  DateTime? _lastAbove;
  DateTime? _cooldownUntil;

  /// Feeds one accelerometer sample in m/s^2, gravity included.
  void onSample(double x, double y, double z, DateTime time) {
    if (isSuppressed?.call() ?? false) {
      _peaks.clear();
      _lastAbove = null;
      return;
    }
    final cooldownUntil = _cooldownUntil;
    if (cooldownUntil != null && time.isBefore(cooldownUntil)) return;

    final gForce = math.sqrt(x * x + y * y + z * z) / _gravity;
    if (gForce <= threshold) return;

    // A sample that follows an above-threshold sample closely is the same
    // jolt, so a long jolt cannot count as several peaks.
    final last = _lastAbove;
    _lastAbove = time;
    if (last != null && time.difference(last) < minPeakSpacing) return;

    _peaks
      ..removeWhere((peak) => time.difference(peak) > window)
      ..add(time);
    if (_peaks.length < peakCount) return;

    _peaks.clear();
    _lastAbove = null;
    _cooldownUntil = time.add(cooldown);
    onShake();
  }
}
