// Gives the app its feedback controller and the screenshot boundary, and
// listens for shakes. Placed inside the app theme and the settings scope, so
// commands and settings rows can reach the controller from the navigator.
//
// Shakes need the phone's accelerometer, so the listener exists only on
// Android and iOS and only while the device setting "Shake to send feedback"
// is on. Tests run on the host platform and never subscribe.

import 'dart:async';
import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:sensors_plus/sensors_plus.dart';

import '../features/settings/settings_store.dart';
import 'feedback_controller.dart';
import 'shake_detector.dart';

class FeedbackHost extends StatefulWidget {
  const FeedbackHost({
    super.key,
    required this.navigatorKey,
    required this.child,
  });

  /// The navigator the sheet opens over (a shake has no context of its own).
  final GlobalKey<NavigatorState> navigatorKey;
  final Widget child;

  /// The controller of the nearest host above [context].
  static FeedbackController of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_FeedbackScope>()!.controller;

  @override
  State<FeedbackHost> createState() => _FeedbackHostState();
}

class _FeedbackHostState extends State<FeedbackHost> {
  final _captureKey = GlobalKey();
  late final _controller = FeedbackController(captureKey: _captureKey);
  AppSettings? _settings;
  StreamSubscription<AccelerometerEvent>? _shakes;

  /// Phones have the accelerometer. Desktop and test hosts do not subscribe.
  static bool get _hasAccelerometer =>
      !kIsWeb && (Platform.isAndroid || Platform.isIOS);

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final settings = AppSettings.read(context);
    if (identical(settings, _settings)) return;
    _settings?.removeListener(_syncShakes);
    _settings = settings..addListener(_syncShakes);
    _syncShakes();
  }

  @override
  void dispose() {
    _settings?.removeListener(_syncShakes);
    _shakes?.cancel();
    super.dispose();
  }

  void _syncShakes() {
    final want = _hasAccelerometer && (_settings?.shakeToSend ?? false);
    if (want == (_shakes != null)) return;
    if (!want) {
      unawaited(_shakes?.cancel());
      _shakes = null;
      return;
    }
    final detector = ShakeDetector(
      onShake: _onShake,
      isSuppressed: () => _controller.isOpen,
    );
    // The default 200 ms sampling misses short shakes; the game interval
    // (20 ms) gives the detector samples close enough to see its peaks.
    _shakes = accelerometerEventStream(
      samplingPeriod: SensorInterval.gameInterval,
    ).listen(
      (e) => detector.onSample(e.x, e.y, e.z, DateTime.now()),
    );
  }

  void _onShake() {
    final context = widget.navigatorKey.currentContext;
    if (context != null) _controller.open(context);
  }

  @override
  Widget build(BuildContext context) => _FeedbackScope(
    controller: _controller,
    child: RepaintBoundary(key: _captureKey, child: widget.child),
  );
}

class _FeedbackScope extends InheritedWidget {
  const _FeedbackScope({required this.controller, required super.child});

  final FeedbackController controller;

  @override
  bool updateShouldNotify(_FeedbackScope old) => old.controller != controller;
}
