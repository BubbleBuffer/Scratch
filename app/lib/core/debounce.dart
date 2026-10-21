// A one-shot debounce timer that counts itself while scheduled, so tests
// can wait for the stores' delayed reloads (the way they wait for bridge
// calls with Vault.inFlight) instead of guessing how long they take.

import 'dart:async';

class Debounce {
  /// Debounces scheduled and not yet run, across the app.
  static int pending = 0;

  /// Tests keep timer scheduling and pending accounting, without real waits.
  static bool instant = false;

  static Duration testDelay(Duration delay) => instant ? Duration.zero : delay;

  Timer? _timer;

  bool get scheduled => _timer != null;

  /// Runs [f] after [delay], replacing a run scheduled earlier.
  void run(Duration delay, void Function() f) {
    cancel();
    pending++;
    _timer = Timer(testDelay(delay), () {
      _timer = null;
      pending--;
      f();
    });
  }

  void cancel() {
    final t = _timer;
    if (t == null) return;
    t.cancel();
    _timer = null;
    pending--;
  }
}
