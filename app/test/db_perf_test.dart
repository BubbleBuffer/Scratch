// Scrolling the 10k-row seed_tasks collection in a table and a board: the
// store loads windows of rows, only visible rows are built, and frames stay
// cheap. Frame times are wall-clock times of a debug-mode test frame (build,
// layout and paint, no rasterising), so they are an upper bound on the work
// a release build does; the budget is loose and the numbers are printed.

@Tags(['perf'])
library;

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:notes_app/state/collection_store.dart';

import 'db_harness.dart';

const rows = 10000;

/// Frame times of real scrolling by [delta], repeated [times].
/// Wheel input avoids starting the desktop board cards' drag-and-drop gesture.
Future<List<double>> scrollFrames(
  WidgetTester tester,
  Finder f,
  Offset delta,
  int times, {
  bool wheel = false,
}) async {
  final frames = <double>[];
  final scrollable = tester.widget(f) is Scrollable
      ? f
      : find.descendant(of: f, matching: find.byType(Scrollable)).first;
  final position = tester.state<ScrollableState>(scrollable).position;
  final start = position.pixels;
  for (var n = 0; n < times; n++) {
    final point = visibleCenter(tester, f);
    final g = wheel ? null : await tester.startGesture(point);
    for (var i = 1; i <= 8; i++) {
      if (wheel) {
        await tester.sendEventToBinding(
          PointerScrollEvent(position: point, scrollDelta: -delta / 8),
        );
      } else {
        await g!.moveBy(delta / 8);
      }
      final sw = Stopwatch()..start();
      await tester.pump(const Duration(milliseconds: 16));
      frames.add(sw.elapsedMicroseconds / 1000);
    }
    await g?.up();
    // Let the windowed query for the new position land.
    await settle(tester);
  }
  expect(
    position.pixels - start,
    greaterThan(10000),
    reason: 'perf must measure real scrolling and window loads',
  );
  return frames;
}

String stats(List<double> f) {
  final s = [...f]..sort();
  double at(double q) => s[math.min(s.length - 1, (q * s.length).floor())];
  return 'frames ${s.length}, median ${at(0.5).toStringAsFixed(1)} ms, p95 ${at(0.95).toStringAsFixed(1)} ms, '
      'max ${s.last.toStringAsFixed(1)} ms (frame ${f.indexOf(s.last)})';
}

int built(String prefix) => find
    .byWidgetPredicate(
      (w) =>
          w.key is ValueKey<String> &&
          (w.key as ValueKey<String>).value.startsWith(prefix),
    )
    .evaluate()
    .length;

void main() {
  setUpAll(initCore);
  tearDownAll(disposeReadOnlySeed);

  dbTest('10k rows scroll in a table', (tester) async {
    await pumpDb(tester, mobile: false, tasks: rows, reuseReadOnlySeed: true);
    await openPage(tester, 'Tasks');
    await settleUntil(tester, () => dbState(tester).store.main.loaded);
    final st = dbState(tester).store;
    expect(st.main.total, rows);
    final frames = await scrollFrames(
      tester,
      byKey('table-rows'),
      const Offset(0, -800),
      60,
    );
    // Deep in the list: rows there are loaded and shown, the rest are not.
    expect(built('row-'), lessThan(80));
    expect(
      st.main.loadedRows.length,
      lessThanOrEqualTo(RowSource.maxWindows * RowSource.window),
    );
    expect(find.text('Loading…'), findsNothing);
    // ignore: avoid_print
    print(
      'table, 10k rows: ${stats(frames)}, rows built ${built('row-')}, loaded ${st.main.loadedRows.length}',
    );
    final s = [...frames]..sort();
    expect(s[(s.length * 0.95).floor()], lessThan(60));
  }, timeout: const Timeout(Duration(minutes: 5)));

  dbTest('10k rows scroll in a board', (tester) async {
    await pumpDb(tester, mobile: false, tasks: rows, reuseReadOnlySeed: true);
    await openPage(tester, 'Tasks');
    await tapKey(tester, 'tab-Board');
    final col = find.byWidgetPredicate(
      (w) =>
          w.key is ValueKey<String> &&
          (w.key as ValueKey<String>).value.startsWith('board-col-'),
    );
    await settleUntil(
      tester,
      () => col.evaluate().isNotEmpty && built('card-') > 0,
    );
    final first = col.first;
    final list = find
        .descendant(of: first, matching: find.byType(Scrollable))
        .first;
    final frames = await scrollFrames(
      tester,
      list,
      const Offset(0, -800),
      60,
      wheel: true,
    );
    expect(built('card-'), lessThan(150));
    // ignore: avoid_print
    print('board, 10k rows: ${stats(frames)}, cards built ${built('card-')}');
    final s = [...frames]..sort();
    expect(s[(s.length * 0.95).floor()], lessThan(60));
  }, timeout: const Timeout(Duration(minutes: 5)));
}
