/// The app clock. Tests fix its starting instant; elapsed time stays real.
abstract final class Clock {
  static DateTime? _fixedAt;
  static final _elapsed = Stopwatch();

  static DateTime? get fixedAt => _fixedAt;

  static set fixedAt(DateTime? value) {
    _fixedAt = value;
    _elapsed
      ..stop()
      ..reset();
    if (value != null) _elapsed.start();
  }
}

DateTime clockNow() => Clock.fixedAt?.add(Clock._elapsed.elapsed) ?? DateTime.now();
