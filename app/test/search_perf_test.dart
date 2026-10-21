// Real Ctrl+K and agent-composer @ search, including text input, the Rust
// bridge, and the frame that displays the current query's results. The cached
// immediate-search control uses the same widgets and result sets to expose
// debug widget-test overhead. These timings do not include GPU rasterisation.
@Tags(['perf'])
library;

import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/features/reminders/backends.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/shell/search.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart' show finishAsync, initCore, loadFonts, resetTestClock;

const _pages = 10000;
const _query = 'apple';
const _typoQueries = ['applr', 'gadr', 'tomatoe'];
const _rounds = 20;
const _uiTargetMs = 20.0;
// Calibrated on 2026-10-09: Ctrl+K real p95 58.208 ms, instantaneous
// search control p95 53.32 ms. The debug harness cannot meet 20 ms even
// without search work. Keep a fixed 3x regression ceiling; the 20 ms UI
// target needs a profile-mode check.
const _ctrlKDebugCeilingMs = 174.624;
// Agent @'s instant control p95 was 16.88 ms initially and 28.40 ms on
// the next run; debug overhead makes a 20 ms gate unstable on this host.
// Calibrated on 2026-10-09 to 3x the latter real p95 of 29.284 ms.
// Keep this fixed regression ceiling; the 20 ms UI target needs a
// profile-mode check for this route as well.
const _mentionDebugCeilingMs = 87.852;

/// Only substituted while a measured search is in progress. All setup,
/// stores, page metadata and disposal use the real bridge handle.
class _SearchHandle implements NotesVault {
  _SearchHandle(this.source, this.cached);
  final NotesVault source;
  final Map<String, SearchResults> cached;
  bool instant = false;
  int started = 0, completed = 0;
  List<Hit> last = const [];

  @override
  Future<List<Hit>> search({required String query, required int limit}) async {
    final request = ++started;
    last = instant
        ? cached[query]!.hits.take(limit).toList()
        : await source.search(query: query, limit: limit);
    completed = request;
    return last;
  }

  @override
  Future<SearchResults> searchAdaptive({
    required String query,
    required int limit,
  }) async {
    final request = ++started;
    final result = instant
        ? SearchResults(
            hits: cached[query]!.hits.take(limit).toList(),
            narrowed: cached[query]!.narrowed,
          )
        : await source.searchAdaptive(query: query, limit: limit);
    last = result.hits;
    completed = request;
    return result;
  }

  // Search-result rows resolve icons and ancestor paths through PagesStore;
  // preserve those real metadata reads in both the control and real route.
  @override
  Future<PageData> loadPage({required String id}) => source.loadPage(id: id);

  // Type chips and highlights use the real parser in both timing routes.
  @override
  Future<ParsedQuery> parseSearch({required String q}) => source.parseSearch(q: q);

  @override
  dynamic noSuchMethod(Invocation invocation) => throw StateError(
    'Unexpected call during search: ${invocation.memberName}',
  );
}

class _SearchVault extends Vault {
  _SearchVault(super.raw, Map<String, SearchResults> cached)
    : probe = _SearchHandle(raw, cached);
  final _SearchHandle probe;
  bool measuring = false;

  @override
  Future<T> run<T>(Future<T> Function(NotesVault) f, {bool writes = false}) =>
      super.run((raw) => f(measuring ? probe : raw), writes: writes);
}

double _quantile(List<double> samples, double q) {
  final sorted = [...samples]..sort();
  return sorted[math.min(sorted.length - 1, (sorted.length * q).ceil() - 1)];
}

String _stats(List<double> samples) =>
    'n=${samples.length}, p50=${_quantile(samples, .5).toStringAsFixed(2)} ms, '
    'p95=${_quantile(samples, .95).toStringAsFixed(2)} ms';

void _report(
  String route,
  List<double> real,
  List<double> control, {
  String query = _query,
}) {
  // ignore: avoid_print
  print('$route real: ${_stats(real)}; instant control: ${_stats(control)}');
  for (var index = 0; index < query.length; index++) {
    List<double> prefixSamples(List<double> samples) => [
      for (var i = index; i < samples.length; i += query.length) samples[i],
    ];
    // ignore: avoid_print
    print(
      '$route ${query.substring(0, index + 1)} real: '
      '${_stats(prefixSamples(real))}; instant control: '
      '${_stats(prefixSamples(control))}',
    );
  }
}

Finder _key(String value) => find.byKey(ValueKey(value));

Future<void> _quiet(WidgetTester tester) async {
  final deadline = Stopwatch()..start();
  for (var i = 0; i < 5 || Vault.inFlight.value > 0; i++) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pump(const Duration(milliseconds: 100));
    if (deadline.elapsed > const Duration(seconds: 10)) {
      throw TestFailure('search fixture did not become idle');
    }
  }
}

Future<List<double>> _typeQuery(
  WidgetTester tester,
  _SearchVault vault, {
  required bool mentions,
  required bool instant,
  String query = _query,
}) async {
  final probe = vault.probe..instant = instant;
  final samples = <double>[];
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
    for (var length = 1; length <= query.length; length++) {
      final prefix = query.substring(0, length);
      final previous = probe.started;
      vault.measuring = true;
      final watch = Stopwatch()..start();
      tester.testTextInput.enterText('${mentions ? '@' : ''}$prefix');
      final deadline = Stopwatch()..start();
      do {
        await tester.runAsync(() => Future<void>.delayed(Duration.zero));
        await tester.pump();
      } while (probe.completed <= previous &&
          deadline.elapsed < const Duration(seconds: 10));
      // Completion is recorded before the widget's await continuation; pump
      // one results frame after it so a previous query's rows cannot qualify.
      await tester.pump();
      watch.stop();
      vault.measuring = false;
      expect(probe.started, previous + 1, reason: 'one search per keystroke');
      expect(probe.completed, probe.started, reason: 'current query completed');
      expect(probe.last, isNotEmpty, reason: 'seed must match every prefix');
      if (_typoQueries.contains(query) && length == query.length) {
        expect(
          probe.last.any((hit) => hit.fuzzy),
          isTrue,
          reason: 'the final typo keystroke must exercise fuzzy search',
        );
      }
      if (mentions) {
        expect(_key('mention-page-${probe.last.first.id}'), findsOneWidget);
      } else {
        final first = tester
            .widgetList<SearchResultRow>(find.byType(SearchResultRow))
            .first;
        expect(first.hit.id, probe.last.first.id);
      }
      if (round >= 0) samples.add(watch.elapsedMicroseconds / 1000);
    }
  }
  return samples;
}

void main() {
  setUpAll(initCore);

  testWidgets('10k pages: Ctrl+K and agent @ keystroke to results', (
    tester,
  ) async {
    resetTestClock();
    await tester.runAsync(loadFonts);
    sansFamily = 'Atkinson Hyperlegible Next';
    monoFamily = 'Fragment Mono';
    EditableText.debugDeterministicCursor = true;
    debugDefaultTargetPlatformOverride = TargetPlatform.windows;
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(1440, 900);
    final dir = Directory.systemTemp.createTempSync('notes_search_perf_');
    AppState? app;
    _SearchVault? vault;
    try {
      final root = '${dir.path}${Platform.pathSeparator}vault';
      final raw = (await tester.runAsync(() async {
        final seed = Stopwatch()..start();
        await NotesVault.seed(root: root, pages: _pages - 1, bigPageBlocks: 0);
        // ignore: avoid_print
        print(
          'search perf fixture: $_pages pages, ${seed.elapsedMilliseconds} ms',
        );
        return NotesVault.open(root: root);
      }))!;
      // ignore: avoid_print
      print('search perf: vault opened');
      final cached = (await tester.runAsync(() async {
        final hits = <String, SearchResults>{};
        for (final query in [_query, ..._typoQueries]) {
          for (var length = 1; length <= query.length; length++) {
            final prefix = query.substring(0, length);
            hits[prefix] = await raw.searchAdaptive(query: prefix, limit: 100);
          }
        }
        return hits;
      }))!;
      // ignore: avoid_print
      print('search perf: query controls cached');
      final measured = vault = _SearchVault(raw, cached);
      expect(await tester.runAsync(measured.pageCount), _pages);
      // Construct the stores in the real zone as well: ReminderStore's
      // initial Future.value must complete while runAsync awaits start.
      final state = (await tester.runAsync(() async {
        final created = app = AppState(
          measured,
          reminderBackend: FakeReminderBackend(),
        );
        await created.start();
        return created;
      }))!;
      // ignore: avoid_print
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
      // Exercise the real registered keyboard shortcut, rather than opening
      // SearchPanel directly.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyK);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await _quiet(tester);
      expect(_key('search-overlay'), findsOneWidget);
      final searchControl = await _typeQuery(
        tester,
        measured,
        mentions: false,
        instant: true,
      );
      // ignore: avoid_print
      print('search perf: Ctrl+K instant control measured');
      final search = await _typeQuery(
        tester,
        measured,
        mentions: false,
        instant: false,
      );
      // ignore: avoid_print
      print('search perf: Ctrl+K native search measured');
      for (final query in _typoQueries) {
        final control = await _typeQuery(
          tester,
          measured,
          mentions: false,
          instant: true,
          query: query,
        );
        final real = await _typeQuery(
          tester,
          measured,
          mentions: false,
          instant: false,
          query: query,
        );
        _report('Ctrl+K typo $query', real, control, query: query);
        expect(
          _quantile(real, .95),
          lessThan(_ctrlKDebugCeilingMs),
          reason: 'typo input retains the fixed Ctrl+K debug ceiling',
        );
      }
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await _quiet(tester);
      expect(_key('search-overlay'), findsNothing);
      final mentionControl = await _typeQuery(
        tester,
        measured,
        mentions: true,
        instant: true,
      );
      // ignore: avoid_print
      print('search perf: agent @ instant control measured');
      final mentions = await _typeQuery(
        tester,
        measured,
        mentions: true,
        instant: false,
      );
      // ignore: avoid_print
      print('search perf: agent @ native search measured');
      for (final query in _typoQueries) {
        final control = await _typeQuery(
          tester,
          measured,
          mentions: true,
          instant: true,
          query: query,
        );
        final real = await _typeQuery(
          tester,
          measured,
          mentions: true,
          instant: false,
          query: query,
        );
        _report('Agent @ typo $query', real, control, query: query);
        expect(
          _quantile(real, .95),
          lessThan(_mentionDebugCeilingMs),
          reason: 'typo input retains the fixed agent @ debug ceiling',
        );
      }
      _report('Ctrl+K', search, searchControl);
      _report('Agent @', mentions, mentionControl);
      // ignore: avoid_print
      print(
        'Search gates: Ctrl+K debug p95 < $_ctrlKDebugCeilingMs ms '
        'and Agent @ debug p95 < $_mentionDebugCeilingMs ms '
        '(fixed calibrations; both profile targets < $_uiTargetMs ms)',
      );
      expect(
        _quantile(search, .95),
        lessThan(_ctrlKDebugCeilingMs),
        reason:
            'Ctrl+K calibration: instant-search p95 53.32 ms, real p95 '
            '58.208 ms; this fixed regression ceiling is 3x that real p95. '
            'The 20 ms UI target needs a profile-mode check.',
      );
      expect(
        _quantile(mentions, .95),
        lessThan(_mentionDebugCeilingMs),
        reason:
            'Agent @ calibration: instant-search p95 16.88 then 28.40 ms, '
            'real p95 29.284 ms; this fixed ceiling is 3x that real p95. '
            'The 20 ms UI target needs a profile-mode check.',
      );
    } finally {
      if (vault != null) vault.measuring = false;
      await tester.pumpWidget(const SizedBox.shrink());
      final closingApp = app;
      if (closingApp != null) await finishAsync(tester, closingApp.dispose());
      await tester.runAsync(() => dir.delete(recursive: true));
      tester.view.reset();
      EditableText.debugDeterministicCursor = false;
      debugDefaultTargetPlatformOverride = null;
    }
  }, timeout: const Timeout(Duration(minutes: 10)));
}
