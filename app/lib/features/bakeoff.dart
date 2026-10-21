// BAKEOFF instrumentation (the Bake-off spec in the internal wiki; current state: wiki/prototypes/flutter.md).
// Off unless the app is built with `--dart-define=BAKEOFF=true`; every call
// is then a no-op.
//
// Everything is logged as lines starting with `BAKEOFF` through print(),
// which reaches `adb logcat` (tag `flutter`) on Android. On desktop the lines
// are also appended to `<data dir>/bakeoff.log`.
//
// - Frame times: SchedulerBinding.addTimingsCallback, build + raster per
//   frame. Only frames whose vsync falls inside a scroll (scroll start to
//   scroll end, plus 100 ms) are counted, so cursor blinks and idle frames do
//   not dilute the numbers. A summary per scroll session (ended by 2.5 s
//   without scrolling) and a running total per screen are logged.
// - Input latency: from the key event (its platform timestamp when the clock
//   matches, otherwise the moment Dart received the text change) to the end
//   of rasterization of the first frame that started after the change.
// - Page open: from the tap on a list row to the end of rasterization of the
//   first frame that painted the page's blocks.

import 'dart:async';
import 'dart:developer' show Timeline;
import 'dart:io';
import 'dart:ui' show FrameTiming, FramePhase;

import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import '../core/clock.dart';

class Bakeoff {
  Bakeoff._();
  static final Bakeoff instance = Bakeoff._();

  /// Built with `--dart-define=BAKEOFF=true`.
  static const enabled = bool.fromEnvironment('BAKEOFF');

  File? _logFile;
  String screen = 'list';

  // Scroll intervals in Timeline.now microseconds.
  int _scrollDepth = 0;
  int _scrollStart = 0;
  final List<(int, int)> _intervals = [];
  final List<double> _segment = [];
  final Map<String, List<double>> _totals = {};
  String _segmentScreen = 'list';
  Timer? _dumpTimer;

  // Input latency.
  final List<(int, int?)> _pendingInputs = []; // (dart receive µs, key event µs)
  final List<double> _inputSegment = [];
  final List<double> _inputTotal = [];
  final List<double> _inputDispatchTotal = [];
  Timer? _inputDumpTimer;
  int? _lastKeyUs;
  int? _lastKeyDartUs;
  bool? _keyClockOk;

  // Page open.
  int? _openStart;
  String _openTitle = '';
  int? _openPaintedAt;
  int _openBlocks = 0;
  double _openLoadMs = 0;

  bool _clockChecked = false;

  void init({String? logDir}) {
    if (!enabled) return;
    if (logDir != null && !Platform.isAndroid) {
      try {
        Directory(logDir).createSync(recursive: true);
        _logFile = File('$logDir${Platform.pathSeparator}bakeoff.log');
      } catch (_) {}
    }
    SchedulerBinding.instance.addTimingsCallback(_onTimings);
    HardwareKeyboard.instance.addHandler(_onKey);
    log('start platform=${Platform.operatingSystem} pid=$pid');
  }

  void log(String msg) {
    if (!enabled) return;
    final line = 'BAKEOFF $msg';
    // ignore: avoid_print
    print(line);
    try {
      _logFile?.writeAsStringSync('${clockNow().toIso8601String()} $line\n',
          mode: FileMode.append);
    } catch (_) {}
  }

  // ---- scrolling ----

  void scrollStarted(String where) {
    if (!enabled) return;
    if (_scrollDepth++ == 0) {
      _scrollStart = Timeline.now;
      if (_segmentScreen != where && _segment.isNotEmpty) _dumpFrames();
      _segmentScreen = where;
      _dumpTimer?.cancel();
    }
  }

  void scrollEnded() {
    if (!enabled) return;
    if (_scrollDepth == 0) return;
    if (--_scrollDepth == 0) {
      _intervals.add((_scrollStart, Timeline.now + 100000));
      if (_intervals.length > 64) _intervals.removeAt(0);
      _dumpTimer?.cancel();
      _dumpTimer = Timer(const Duration(milliseconds: 2500), _dumpFrames);
    }
  }

  bool _inScroll(int us) {
    if (_scrollDepth > 0 && us >= _scrollStart) return true;
    for (var i = _intervals.length - 1; i >= 0; i--) {
      final (a, b) = _intervals[i];
      if (us >= a && us <= b) return true;
    }
    return false;
  }

  void _onTimings(List<FrameTiming> timings) {
    if (!_clockChecked && timings.isNotEmpty) {
      _clockChecked = true;
      final lag = Timeline.now - timings.last.timestampInMicroseconds(FramePhase.rasterFinish);
      log('clock rasterFinish-lag-us=$lag (small positive means Timeline.now and FrameTiming share a clock)');
    }
    var gotInput = false;
    for (final t in timings) {
      final vsync = t.timestampInMicroseconds(FramePhase.vsyncStart);
      final buildStart = t.timestampInMicroseconds(FramePhase.buildStart);
      final rasterFinish = t.timestampInMicroseconds(FramePhase.rasterFinish);
      if (_inScroll(vsync)) {
        final ms = (t.buildDuration + t.rasterDuration).inMicroseconds / 1000.0;
        _segment.add(ms);
        (_totals[_segmentScreen] ??= []).add(ms);
      }
      // Input latency: first frame whose build started after the change.
      while (_pendingInputs.isNotEmpty && buildStart >= _pendingInputs.first.$1) {
        final (dartUs, keyUs) = _pendingInputs.removeAt(0);
        final fromDispatch = (rasterFinish - dartUs) / 1000.0;
        final fromKey = keyUs != null ? (rasterFinish - keyUs) / 1000.0 : fromDispatch;
        _inputSegment.add(fromKey);
        _inputTotal.add(fromKey);
        _inputDispatchTotal.add(fromDispatch);
        gotInput = true;
      }
      final painted = _openPaintedAt;
      if (painted != null && t.timestampInMicroseconds(FramePhase.buildFinish) >= painted) {
        final ms = (rasterFinish - _openStart!) / 1000.0;
        log('page-open ms=${ms.toStringAsFixed(1)} title="$_openTitle" blocks=$_openBlocks '
            'core-load-ms=${_openLoadMs.toStringAsFixed(1)}');
        _openPaintedAt = null;
        _openStart = null;
      }
    }
    // Only new input restarts the dump timer: the blinking caret paints a
    // frame every 500 ms, which would otherwise postpone the dump forever.
    if (gotInput) {
      _inputDumpTimer?.cancel();
      _inputDumpTimer = Timer(const Duration(milliseconds: 2500), _dumpInput);
    }
  }

  void _dumpFrames() {
    if (_segment.isEmpty) return;
    log('frames screen=$_segmentScreen ${_stats(_segment, true)}');
    final tot = _totals[_segmentScreen]!;
    log('frames-total screen=$_segmentScreen ${_stats(tot, true)}');
    _segment.clear();
  }

  // ---- typing ----

  bool _onKey(KeyEvent e) {
    if (e is KeyDownEvent || e is KeyRepeatEvent) {
      _lastKeyUs = e.timeStamp.inMicroseconds;
      _lastKeyDartUs = Timeline.now;
    }
    return false;
  }

  /// Called by the editor when a block's text changed through input.
  void textChanged() {
    if (!enabled) return;
    final now = Timeline.now;
    int? keyUs;
    final lk = _lastKeyUs, ld = _lastKeyDartUs;
    if (lk != null && ld != null && now - ld < 200000) {
      final delta = now - lk;
      _keyClockOk ??= delta >= 0 && delta < 1000000;
      if (_keyClockOk == false && delta >= 0 && delta < 1000000) _keyClockOk = true;
      if (_keyClockOk! && delta >= 0 && delta < 1000000) keyUs = lk;
    }
    _lastKeyUs = null;
    _pendingInputs.add((now, keyUs));
    if (_pendingInputs.length > 50) _pendingInputs.removeAt(0);
  }

  void _dumpInput() {
    if (_inputSegment.isEmpty) return;
    log('input ${_stats(_inputSegment, false)} key-clock=${_keyClockOk ?? 'none'}');
    log('input-total ${_stats(_inputTotal, false)} '
        'from-dart-dispatch: ${_stats(_inputDispatchTotal, false)}');
    _inputSegment.clear();
  }

  // ---- page open ----

  void pageOpenStart(String title) {
    if (!enabled) return;
    _openStart = Timeline.now;
    _openTitle = title;
    _openPaintedAt = null;
  }

  void pageBlocksBuilt(int blocks, double loadMs) {
    if (!enabled) return;
    if (_openStart == null) return;
    _openBlocks = blocks;
    _openLoadMs = loadMs;
    _openPaintedAt = Timeline.now;
  }

  // ---- helpers ----

  static String _stats(List<double> xs, bool frames) {
    final s = [...xs]..sort();
    double pct(double p) => s[((s.length - 1) * p).round()];
    final over = s.where((x) => x > 16.7).length * 100.0 / s.length;
    final b = StringBuffer('n=${s.length} p50=${pct(.5).toStringAsFixed(2)} '
        'p95=${pct(.95).toStringAsFixed(2)} p99=${pct(.99).toStringAsFixed(2)} '
        'max=${s.last.toStringAsFixed(2)}');
    if (frames) b.write(' over16.7=${over.toStringAsFixed(1)}%');
    return b.toString();
  }
}
