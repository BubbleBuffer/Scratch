// Profile-mode timing of the real Ctrl+K and agent-composer @ search, on
// Windows or Linux. Same fixture, query, rounds and correctness checks as
// test/search_perf_test.dart, but built in profile mode, run through
// `flutter drive` and loaded with the release bridge named by NOTES_BRIDGE_LIB.
// The 20 ms UI target is judged from the reported numbers; nothing asserts it.
//
// Run from app/ after the release bridge is built (cargo build -p notes_bridge
// --release), with NOTES_BRIDGE_LIB set to the absolute path of the bridge
// (target/release/notes_bridge.dll on Windows, libnotes_bridge.so on Linux):
//
//   flutter drive --profile -d linux \
//     --driver=test_driver/search_profile_driver.dart \
//     --target=integration_test/search_profile_test.dart
// Use -d windows for the Windows desktop run.
//
// Timing: enterText starts the stopwatch; it stops after the bridge completes
// and the next frame is pumped. Each sample is split on that same stopwatch:
//   beforeBridge: enterText to the search call reaching the bridge handle
//   bridge:       the awaited bridge call (FRB round trip, includes Rust time)
//   afterBridge:  bridge completion to the results frame being pumped
// nonBridge is total - bridge. allBridge is the union of every handle call
// (search, parseSearch, loadPage) pending in the window, clipped to the
// endpoint; outsideBridge is total - allBridge. These are wall-time
// partitions: calls can overlap each other and Flutter work, so outsideBridge
// is not Flutter-only CPU time. The live binding paints only frames the test
// pumps (explicit onlyPumps), and each sample waits up to one vsync interval
// (refreshRateHz is reported). build/raster come from engine FrameTiming of
// the frame whose build starts inside the sample window. Raster runs on the
// raster thread and may overlap the pump; total does not await its completion.
// inputToRasterFinishMs separately measures the matched frame's raster endpoint.
// Results go to responseData; test_driver prints the stage summary and writes
// build/search_perf/.

// This is a CLI-facing harness; we print results as part of the interface.
// ignore_for_file: avoid_print

import 'dart:developer' show Timeline;
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' show FramePhase;

import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/shell/addons.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/shell/search.dart';
import 'package:notes_app/src/rust/api/clock.dart' as rust_clock;
import 'package:notes_app/src/rust/frb_generated.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/ui/ui.dart';

const _pages = 10000;
const _query = 'apple';
const _rounds = 20;
const _uiTargetMs = 20.0;
const _keystrokeTimeout = Duration(seconds: 10);
const _bridgeEnv = 'NOTES_BRIDGE_LIB';
const _semantics = [
  'total: enterText to results frame pumped (the existing stopwatch endpoint)',
  'beforeBridge: enterText to the search call reaching the bridge handle',
  'bridge: awaited bridge call (FRB round trip, includes Rust time)',
  'afterBridge: bridge completion to results frame pumped',
  'nonBridge: total - bridge (= beforeBridge + afterBridge)',
  'allBridge: union of every handle-call interval (search, parseSearch, '
      'loadPage), clipped to the endpoint',
  'outsideBridge: total - allBridge',
  'allBridge and outsideBridge are wall-time partitions, not CPU attribution: '
      'handle calls overlap each other and Flutter work, so outsideBridge is '
      'time with no handle call pending, not Flutter-only time',
  'buildMs/rasterMs: engine FrameTiming of the frame whose build starts in the '
      'sample window; total does not await raster completion',
  'inputToRasterFinishMs: Timeline start to matched frame raster finish; '
      'includes raster completion, not physical display presentation',
  'bridgeToBuildStartMs: main search completion to matched frame build start; '
      'includes event-loop scheduling and waiting for vsync',
];
// Same instant as the debug harness, so the core and app clocks match.
final _fixedNow = DateTime(2026, 10, 6, 11, 20);

// Bundled faces are declared under `fonts:` in pubspec.yaml and read from the
// asset bundle here, so the profile build does not depend on the working dir.
const _bundledFonts = <String, List<String>>{
  'Atkinson Hyperlegible Next': [
    'assets/fonts/AtkinsonHyperlegibleNext[wght].ttf',
    'assets/fonts/AtkinsonHyperlegibleNext-Italic[wght].ttf',
  ],
  'Bricolage Grotesque': [
    'assets/fonts/BricolageGrotesque[opsz,wdth,wght].ttf',
  ],
  'Fragment Mono': [
    'assets/fonts/FragmentMono-Regular.ttf',
    'assets/fonts/FragmentMono-Italic.ttf',
  ],
};

String? _loadedBridge;

/// The release bridge named by NOTES_BRIDGE_LIB. There is no fallback path:
/// a profile run must load exactly the library the lead built.
String _bridgeLibrary() {
  final path = Platform.environment[_bridgeEnv];
  if (path == null || path.isEmpty) {
    throw StateError('$_bridgeEnv must name the release notes_bridge library');
  }
  final file = File(path);
  if (!file.existsSync()) {
    throw StateError('$_bridgeEnv does not exist: $path');
  }
  return file.absolute.path;
}

Future<void> _loadFonts() async {
  for (final entry in _bundledFonts.entries) {
    final loader = FontLoader(entry.key);
    for (final path in entry.value) {
      loader.addFont(rootBundle.load(path));
    }
    await loader.load();
  }
  // Segoe UI Emoji is a Windows system face; Linux uses the engine fallback.
  if (!Platform.isWindows) return;
  final winDir = Platform.environment['WINDIR'] ?? r'C:\Windows';
  final emoji = File('$winDir\\Fonts\\seguiemj.ttf');
  if (emoji.existsSync()) {
    final loader = FontLoader('Segoe UI Emoji')
      ..addFont(Future.value(ByteData.sublistView(await emoji.readAsBytes())));
    await loader.load();
  } else {
    print('search perf: Segoe UI Emoji not found; using engine fallback');
  }
}

/// Only substituted while a measured keystroke is in flight. Setup, stores,
/// page metadata and disposal use the real bridge handle.
class _SearchHandle implements NotesVault {
  _SearchHandle(this.source);
  final NotesVault source;
  int started = 0, completed = 0;
  List<Hit> last = const [];
  // The current keystroke's stopwatch, and the primary search call's start and
  // end on it (microseconds). Only set while a measured keystroke is in flight.
  Stopwatch? stopwatch;
  int callStartUs = -1, callEndUs = -1;
  // Every handle call made during the keystroke, with its interval on
  // [stopwatch]. Calls can overlap each other and the Flutter work around them.
  final List<_Span> spans = [];

  /// Runs one handle call. [primary] marks the search call that defines the
  /// bridgeMs stage; every call is also recorded as a span.
  Future<T> _span<T>(
    String kind,
    Future<T> Function() call, {
    bool primary = false,
  }) async {
    final watch = stopwatch;
    final span = watch == null ? null : _Span(kind, watch.elapsedMicroseconds);
    if (span != null) spans.add(span);
    if (watch != null && primary && callStartUs < 0) {
      callStartUs = span!.startUs;
    }
    final result = await call();
    if (span != null) span.endUs = watch!.elapsedMicroseconds;
    if (watch != null && primary) callEndUs = span!.endUs;
    return result;
  }

  @override
  Future<List<Hit>> search({required String query, required int limit}) async {
    final request = ++started;
    last = await _span(
      'search',
      () => source.search(query: query, limit: limit),
      primary: true,
    );
    completed = request;
    return last;
  }

  @override
  Future<SearchResults> searchAdaptive({
    required String query,
    required int limit,
  }) async {
    final request = ++started;
    final result = await _span(
      'search',
      () => source.searchAdaptive(query: query, limit: limit),
      primary: true,
    );
    last = result.hits;
    completed = request;
    return result;
  }

  // Search-result rows resolve icons and ancestor paths through PagesStore;
  // keep those real metadata reads inside the measured window.
  @override
  Future<PageData> loadPage({required String id}) =>
      _span('loadPage', () => source.loadPage(id: id));

  // Type chips and highlights use the real parser during measurement.
  @override
  Future<ParsedQuery> parseSearch({required String q}) =>
      _span('parseSearch', () => source.parseSearch(q: q));

  @override
  dynamic noSuchMethod(Invocation invocation) => throw StateError(
    'Unexpected call during search: ${invocation.memberName}',
  );
}

class _SearchVault extends Vault {
  _SearchVault(super.raw) : probe = _SearchHandle(raw);
  final _SearchHandle probe;
  bool measuring = false;

  @override
  Future<T> run<T>(Future<T> Function(NotesVault) f, {bool writes = false}) =>
      super.run((raw) => f(measuring ? probe : raw), writes: writes);
}

/// One handle call's interval on the keystroke stopwatch. endUs is -1 while
/// the call is still pending; it is clipped to the sample endpoint then.
class _Span {
  _Span(this.kind, this.startUs);
  final String kind;
  final int startUs;
  int endUs = -1;
}

const _callKinds = ['search', 'parseSearch', 'loadPage'];

/// Length of the union of the spans' intervals, clipped to [0, limitUs].
/// Overlapping spans count once, so this is wall time with a bridge call
/// pending, not the sum of call durations.
int _unionUs(List<_Span> spans, int limitUs) {
  final ranges = [
    for (final span in spans)
      (
        math.min(math.max(span.startUs, 0), limitUs),
        math.min(span.endUs < 0 ? limitUs : span.endUs, limitUs),
      ),
  ]..sort((a, b) => a.$1.compareTo(b.$1));
  var total = 0;
  int? start, end;
  for (final (s, e) in ranges) {
    if (e <= s) continue;
    if (end == null || s > end) {
      if (end != null) total += end - start!;
      start = s;
      end = e;
    } else if (e > end) {
      end = e;
    }
  }
  if (end != null) total += end - start!;
  return total;
}

/// One measured keystroke. Times in ms; startUs/endUs are Timeline clock
/// microseconds, used to match engine frames to the window.
class _Sample {
  _Sample({
    required this.totalMs,
    required this.beforeBridgeMs,
    required this.bridgeMs,
    required this.afterBridgeMs,
    required this.allBridgeMs,
    required this.outsideBridgeMs,
    required this.callCounts,
    required this.startUs,
    required this.endUs,
  });
  final double totalMs, beforeBridgeMs, bridgeMs, afterBridgeMs;
  // Union of all pending handle-call intervals, and total minus that union.
  final double allBridgeMs, outsideBridgeMs;
  // Handle calls made in this keystroke, per kind (see _callKinds).
  final Map<String, int> callCounts;
  final int startUs, endUs;
  double? buildMs, rasterMs, inputToRasterFinishMs, bridgeToBuildStartMs;
  int frames = 0;
}

double _quantile(List<double> samples, double q) {
  final sorted = [...samples]..sort();
  return sorted[math.min(sorted.length - 1, (sorted.length * q).ceil() - 1)];
}

Map<String, dynamic> _summary(List<double> samples) => samples.isEmpty
    ? {'n': 0, 'p50Ms': null, 'p95Ms': null, 'samplesMs': <double>[]}
    : {
        'n': samples.length,
        'p50Ms': _quantile(samples, .5),
        'p95Ms': _quantile(samples, .95),
        'samplesMs': samples,
      };

/// Route totals, stage and frame summaries, plus one entry per typed prefix
/// (samples are round-major).
Map<String, dynamic> _routeReport(List<_Sample> samples) {
  final totals = [for (final s in samples) s.totalMs];
  final builds = [for (final s in samples) ?s.buildMs];
  final rasters = [for (final s in samples) ?s.rasterMs];
  return {
    ..._summary(totals),
    'stages': {
      'beforeBridgeMs': _summary([for (final s in samples) s.beforeBridgeMs]),
      'bridgeMs': _summary([for (final s in samples) s.bridgeMs]),
      'afterBridgeMs': _summary([for (final s in samples) s.afterBridgeMs]),
      'nonBridgeMs': _summary([
        for (final s in samples) s.totalMs - s.bridgeMs,
      ]),
      'allBridgeMs': _summary([for (final s in samples) s.allBridgeMs]),
      'outsideBridgeMs': _summary([for (final s in samples) s.outsideBridgeMs]),
      'buildMs': _summary(builds),
      'rasterMs': _summary(rasters),
      'inputToRasterFinishMs': _summary([
        for (final s in samples) ?s.inputToRasterFinishMs,
      ]),
      'bridgeToBuildStartMs': _summary([
        for (final s in samples) ?s.bridgeToBuildStartMs,
      ]),
    },
    // Raw per-sample handle-call counts, round-major, one array per kind.
    'calls': {
      for (final kind in _callKinds)
        kind: [for (final s in samples) s.callCounts[kind] ?? 0],
    },
    'frames': {
      'samples': samples.length,
      'matched': builds.length,
      'maxPerSample': samples.fold<int>(0, (m, s) => math.max(m, s.frames)),
    },
    'prefixes': {
      for (var index = 0; index < _query.length; index++)
        _query.substring(0, index + 1): _summary([
          for (var i = index; i < totals.length; i += _query.length) totals[i],
        ]),
    },
  };
}

void _printRoute(String route, List<_Sample> samples) {
  final totals = [for (final s in samples) s.totalMs];
  print(
    'search perf $route: n=${totals.length}, '
    'p50=${_quantile(totals, .5).toStringAsFixed(2)} ms, '
    'p95=${_quantile(totals, .95).toStringAsFixed(2)} ms',
  );
}

/// Pairs each sample with the engine frame whose build starts inside its
/// window. Frame timestamps and Timeline.now are both monotonic microseconds;
/// samples with no matching frame keep null frame fields.
void _matchFrames(List<_Sample> samples, List<FrameTiming> timings) {
  for (final sample in samples) {
    for (final timing in timings) {
      final buildStart = timing.timestampInMicroseconds(FramePhase.buildStart);
      if (buildStart < sample.startUs || buildStart > sample.endUs) continue;
      sample
        ..frames += 1
        ..buildMs = timing.buildDuration.inMicroseconds / 1000
        ..rasterMs = timing.rasterDuration.inMicroseconds / 1000
        ..inputToRasterFinishMs =
            (timing.timestampInMicroseconds(FramePhase.rasterFinish) -
                sample.startUs) /
            1000
        ..bridgeToBuildStartMs =
            (buildStart - sample.startUs) / 1000 -
            sample.beforeBridgeMs -
            sample.bridgeMs;
    }
  }
}

Finder _key(String value) => find.byKey(ValueKey(value));

String? _loadAverage() => Platform.isLinux
    ? File('/proc/loadavg')
          .readAsStringSync()
          .trim()
          .split(' ')
          .take(3)
          .join(' ')
    : null;

/// One real frame. A live pump waits for the engine's next vsync, so a hidden
/// or minimised window would stall it; fail with a message instead of hanging.
Future<void> _frame(WidgetTester tester) => tester.pump().timeout(
  const Duration(seconds: 10),
  onTimeout: () =>
      throw TestFailure('no frame within 10 s (is the window visible?)'),
);

Future<void> _quiet(WidgetTester tester) async {
  final deadline = Stopwatch()..start();
  for (var i = 0; i < 5 || Vault.inFlight.value > 0; i++) {
    await Future<void>.delayed(Duration.zero);
    await tester.pump(const Duration(milliseconds: 100));
    if (deadline.elapsed > const Duration(seconds: 10)) {
      throw TestFailure('search fixture did not become idle');
    }
  }
}

Future<List<_Sample>> _typeQuery(
  WidgetTester tester,
  _SearchVault vault, {
  required bool mentions,
}) async {
  final probe = vault.probe;
  final samples = <_Sample>[];
  final field = mentions
      ? _key('agent-composer')
      : find.descendant(
          of: _key('search-overlay-field'),
          matching: find.byType(EditableText),
        );
  await tester.showKeyboard(field);
  // An unmeasured round warms every prefix, the bridge and both widget trees.
  for (var round = -1; round < _rounds; round++) {
    vault.measuring = false;
    tester.testTextInput.enterText(mentions ? '@' : '');
    await _quiet(tester);
    for (var length = 1; length <= _query.length; length++) {
      final prefix = _query.substring(0, length);
      final previous = probe.started;
      vault.measuring = true;
      final watch = Stopwatch()..start();
      final startUs = Timeline.now;
      probe.stopwatch = watch;
      probe.callStartUs = -1;
      probe.callEndUs = -1;
      probe.spans.clear();
      tester.testTextInput.enterText('${mentions ? '@' : ''}$prefix');
      final deadline = Stopwatch()..start();
      while (probe.completed <= previous) {
        if (deadline.elapsed > _keystrokeTimeout) {
          throw TestFailure(
            'search for "$prefix" did not complete within '
            '${_keystrokeTimeout.inSeconds} s',
          );
        }
        await Future<void>.delayed(Duration.zero);
      }
      // Completion is recorded before the widget's await continuation; the
      // pumped frame draws the results that continuation set.
      await _frame(tester);
      watch.stop();
      final endUs = Timeline.now;
      probe.stopwatch = null;
      vault.measuring = false;
      expect(probe.started, previous + 1, reason: 'one search per keystroke');
      expect(probe.completed, probe.started, reason: 'current query completed');
      expect(probe.last, isNotEmpty, reason: 'seed must match every prefix');
      if (mentions) {
        expect(_key('mention-page-${probe.last.first.id}'), findsOneWidget);
      } else {
        final first = tester
            .widgetList<SearchResultRow>(find.byType(SearchResultRow))
            .first;
        expect(first.hit.id, probe.last.first.id);
      }
      if (round >= 0) {
        final totalUs = watch.elapsedMicroseconds;
        final bridgeUs = probe.callEndUs - probe.callStartUs;
        final allBridgeUs = _unionUs(probe.spans, totalUs);
        samples.add(
          _Sample(
            totalMs: totalUs / 1000,
            beforeBridgeMs: probe.callStartUs / 1000,
            bridgeMs: bridgeUs / 1000,
            afterBridgeMs: (totalUs - probe.callEndUs) / 1000,
            allBridgeMs: allBridgeUs / 1000,
            outsideBridgeMs: (totalUs - allBridgeUs) / 1000,
            callCounts: {
              for (final kind in _callKinds)
                kind: probe.spans.where((s) => s.kind == kind).length,
            },
            startUs: startUs,
            endUs: endUs,
          ),
        );
      }
    }
  }
  return samples;
}

/// Records one route. Engine frame timings arrive asynchronously, so a few
/// extra pumps (outside every sample window) flush the last frames' timings
/// before they are matched. Assigned right away so partial results survive a
/// later failure.
Future<void> _finishRoute(
  WidgetTester tester,
  String key,
  String label,
  List<_Sample> samples,
  List<FrameTiming> timings,
  Map<String, dynamic> routes,
) async {
  for (var i = 0; i < 3; i++) {
    await _frame(tester);
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
  _matchFrames(samples, timings);
  // Clock check: if frame timestamps are not on the Timeline clock, matched
  // stays 0 and this pair shows the offset.
  final lastBuildStart = timings.isEmpty
      ? null
      : timings.last.timestampInMicroseconds(FramePhase.buildStart);
  routes[key] = {
    ..._routeReport(samples),
    'clock': {
      'timelineNowUs': Timeline.now,
      'lastBuildStartUs': lastBuildStart,
    },
  };
  _printRoute(label, samples);
}

Future<void> _deleteDir(Directory dir) async {
  // The bridge's background threads can hold files for a moment after close
  // on Windows, so retry briefly before reporting the failure.
  for (var attempt = 1; ; attempt++) {
    try {
      if (dir.existsSync()) await dir.delete(recursive: true);
      return;
    } on FileSystemException {
      if (attempt >= 10) rethrow;
      await Future<void>.delayed(const Duration(milliseconds: 200));
    }
  }
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized()
    ..framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.onlyPumps;

  setUpAll(() async {
    _loadedBridge = _bridgeLibrary();
    await RustLib.init(externalLibrary: ExternalLibrary.open(_loadedBridge!));
    print('search perf: bridge loaded from $_loadedBridge');
    rust_clock.setTestClock(startMs: _fixedNow.millisecondsSinceEpoch);
    registerBuiltins();
  });

  testWidgets('10k pages: profile Ctrl+K and agent @ keystroke to results', (
    tester,
  ) async {
    expect(kProfileMode, isTrue, reason: 'run with flutter drive --profile');
    expect(
      Platform.isWindows || Platform.isLinux,
      isTrue,
      reason: 'this harness runs on Windows or Linux',
    );

    // Shared by reference: partial results still reach the driver on failure.
    final report = <String, dynamic>{
      'status': 'failed',
      'mode': 'profile',
      'platform': Platform.operatingSystem,
      'osVersion': Platform.operatingSystemVersion,
      'bridge': _loadedBridge,
      'query': _query,
      'rounds': _rounds,
      'warmupRounds': 1,
      'uiTargetMs': _uiTargetMs,
      'run': Platform.environment['SEARCH_PROFILE_RUN'],
      'sdk': {
        'flutter': Platform.environment['SEARCH_PROFILE_SDK'],
        'dart': Platform.version,
      },
      'refreshRateHz': tester.view.display.refreshRate,
      'semantics': _semantics,
    };
    final routes = <String, dynamic>{};
    report['routes'] = routes;
    binding.reportData = report;

    // Mirrors the debug harness' fixed clock and immediate debounces.
    Clock.fixedAt = _fixedNow;
    Debounce.instant = true;
    Debounce.pending = 0;
    await _loadFonts();
    sansFamily = 'Atkinson Hyperlegible Next';
    monoFamily = 'Fragment Mono';
    EditableText.debugDeterministicCursor = true;
    // Keep the debug fixture's logical viewport, but leave the native physical
    // size intact. Faking it would make the engine discard every layer tree
    // whose size differs from the real window, producing zero raster timings.
    await binding.setSurfaceSize(const Size(1440, 900));
    // The integration binding does not register the text input mock, and an
    // unregistered mock would let the real IME attach to the window.
    tester.testTextInput.register();

    // Every engine frame timing for the run; each route matches its own window.
    final frameTimings = <FrameTiming>[];
    void collectTimings(List<FrameTiming> batch) => frameTimings.addAll(batch);
    SchedulerBinding.instance.addTimingsCallback(collectTimings);

    final dir = Directory.systemTemp.createTempSync('notes_search_profile_');
    AppState? app;
    _SearchVault? vault;
    try {
      final root = '${dir.path}${Platform.pathSeparator}vault';
      final seed = Stopwatch()..start();
      await NotesVault.seed(root: root, pages: _pages - 1, bigPageBlocks: 0);
      report['fixture'] = {'pages': _pages, 'seedMs': seed.elapsedMilliseconds};
      print(
        'search perf fixture: $_pages pages, ${seed.elapsedMilliseconds} ms',
      );
      final raw = await NotesVault.open(root: root);
      print('search perf: vault opened');

      final measured = vault = _SearchVault(raw);
      expect(await measured.pageCount(), _pages);
      // Stores are created on the real event loop, as in the debug harness.
      final state = app = AppState(
        measured,
        reminderBackend: FakeReminderBackend(),
      );
      await state.start();
      print('search perf: app stores started');
      final store = AgentStore.of(state);
      await tester.pumpWidget(
        MediaQuery.fromView(
          view: tester.view,
          child: Directionality(
            textDirection: TextDirection.ltr,
            child: NotesShell(
              navigatorKey: GlobalKey<NavigatorState>(),
              wrap: (child) => SettingsScope(
                settings: state.settings,
                child: AppScope(state: state, child: child),
              ),
              home: Center(
                child: SizedBox(width: 480, child: AgentComposer(store: store)),
              ),
            ),
          ),
        ),
      );
      await _quiet(tester);

      report['surface'] = {
        'physicalWidth': tester.view.physicalSize.width,
        'physicalHeight': tester.view.physicalSize.height,
        'devicePixelRatio': tester.view.devicePixelRatio,
        'logicalWidth': 1440,
        'logicalHeight': 900,
      };

      // The registered Ctrl+K shortcut, not SearchPanel opened directly.
      // The simulator's default physical-key lookup uses debug names, which
      // are unavailable in profile mode. Supply the real physical keys.
      await tester.sendKeyDownEvent(
        LogicalKeyboardKey.controlLeft,
        physicalKey: PhysicalKeyboardKey.controlLeft,
      );
      await tester.sendKeyEvent(
        LogicalKeyboardKey.keyK,
        physicalKey: PhysicalKeyboardKey.keyK,
      );
      await tester.sendKeyUpEvent(
        LogicalKeyboardKey.controlLeft,
        physicalKey: PhysicalKeyboardKey.controlLeft,
      );
      await _quiet(tester);
      expect(_key('search-overlay'), findsOneWidget);
      report['measurementsStartedAtUtc'] = DateTime.now()
          .toUtc()
          .toIso8601String();
      print('search profile: measurements begin');
      final searchLoadBefore = _loadAverage();
      final search = await _typeQuery(tester, measured, mentions: false);
      final searchLoadAfter = _loadAverage();
      expect(search, hasLength(_rounds * _query.length));
      await _finishRoute(
        tester,
        'ctrlK',
        'Ctrl+K',
        search,
        frameTimings,
        routes,
      );
      (routes['ctrlK'] as Map<String, dynamic>)['loadAverage'] = {
        'before': searchLoadBefore,
        'after': searchLoadAfter,
      };

      await tester.sendKeyEvent(
        LogicalKeyboardKey.escape,
        physicalKey: PhysicalKeyboardKey.escape,
      );
      await _quiet(tester);
      expect(_key('search-overlay'), findsNothing);
      final mentionsLoadBefore = _loadAverage();
      final mentions = await _typeQuery(tester, measured, mentions: true);
      final mentionsLoadAfter = _loadAverage();
      expect(mentions, hasLength(_rounds * _query.length));
      await _finishRoute(
        tester,
        'agentAt',
        'Agent @',
        mentions,
        frameTimings,
        routes,
      );
      (routes['agentAt'] as Map<String, dynamic>)['loadAverage'] = {
        'before': mentionsLoadBefore,
        'after': mentionsLoadAfter,
      };
      report['measurementsFinishedAtUtc'] = DateTime.now()
          .toUtc()
          .toIso8601String();
      print('Search profile target: p95 < $_uiTargetMs ms (measurement only)');
    } finally {
      vault?.measuring = false;
      SchedulerBinding.instance.removeTimingsCallback(collectTimings);
      await tester.pumpWidget(const SizedBox.shrink());
      final closingApp = app;
      // AppState.dispose also disposes the vault; the call below then no-ops.
      if (closingApp != null) await closingApp.dispose();
      vault?.dispose();
      await _deleteDir(dir);
      tester.testTextInput.unregister();
      await binding.setSurfaceSize(null);
      tester.view.reset();
      EditableText.debugDeterministicCursor = false;
      Clock.fixedAt = null;
      Debounce.instant = false;
    }
    report['status'] = 'passed';
  }, timeout: const Timeout(Duration(minutes: 10)));
}
