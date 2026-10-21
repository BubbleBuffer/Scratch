// What one real edit costs while the 10k-row Tasks collection is in the vault.
// Each edit types one character at the end of an ordinary page's first block,
// then waits until the app is idle again (the 400 ms reload debounce has fired
// and the reload, the change feed and the bridge are done). Prints one
// EDIT_PERF summary line per metric. No limit yet; only the edits must persist.
//
// Metrics:
// - wall_ms: real time from the keystroke until idle (settle's test).
// - store_notifications: PagesStore notifications during the edit (listener on
//   db.app.pages). Each list chunk loaded by the store notifies once, so this
//   stands in for the chunk count; the rows listed are not counted.
// - bridge_calls: bridge calls started during the edit, all kinds (increments
//   of Vault.inFlight). A proxy only: list_pages cannot be told apart from the
//   other calls without changing lib code.
// - frame_ms: wall time of each test frame that had work scheduled (build,
//   layout, paint in debug mode; no rasterising), pooled over all edits.

@Tags(['perf'])
library;

import 'dart:math' as math;

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/vault.dart';

import 'db_harness.dart';
import 'harness.dart' show editableWith, editState;

const taskRows = 10000;
const edits = 20;
const pageTitle = 'Weekly review';
const pageText = 'Some text about weekly review.';
const frameStep = Duration(milliseconds: 50);

/// [settle], with each frame that had work scheduled timed. Same idle test.
Future<List<double>> settleTimed(WidgetTester tester) async {
  final frames = <double>[];
  var quiet = 0;
  var elapsed = Duration.zero;
  final deadline = Stopwatch()..start();
  await tester.runAsync(() async {
    await db.app.feed.poll();
  });
  while (deadline.elapsed < const Duration(seconds: 30)) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    final hadFrame = tester.binding.hasScheduledFrame;
    final sw = Stopwatch()..start();
    await tester.pump(frameStep);
    elapsed += frameStep;
    if (hadFrame) frames.add(sw.elapsedMicroseconds / 1000);
    final dataBusy =
        Vault.inFlight.value > 0 || !db.app.feed.idle || Debounce.pending > 0;
    final busy = dataBusy || tester.binding.hasScheduledFrame;
    quiet = busy ? 0 : quiet + 1;
    if (quiet >= 3) return frames;
    if (!dataBusy && elapsed >= const Duration(seconds: 30)) return frames;
  }
  throw TestFailure(
    'edit did not settle: ${Vault.inFlight.value} bridge calls, '
    '${Debounce.pending} debounces pending',
  );
}

/// One summary line: p50, p95 and max over [values].
String summary(String name, List<double> values, String unit) {
  if (values.isEmpty) return 'EDIT_PERF $name n=0';
  final s = [...values]..sort();
  double at(double q) => s[math.min(s.length - 1, (q * s.length).floor())];
  return 'EDIT_PERF $name p50=${at(0.5).toStringAsFixed(1)}$unit '
      'p95=${at(0.95).toStringAsFixed(1)}$unit '
      'max=${s.last.toStringAsFixed(1)}$unit n=${s.length}';
}

void main() {
  setUpAll(initCore);
  tearDownAll(disposeReadOnlySeed);

  dbTest(
    'edits in an ordinary page with the 10k-row vault',
    (tester) async {
      await pumpDb(
        tester,
        mobile: false,
        tasks: taskRows,
        reuseReadOnlySeed: true,
      );
      await openPage(tester, pageTitle);
      // Let the store's first load finish before the first measured edit.
      await settleUntil(
        tester,
        () => db.app.pages.count >= taskRows,
        timeout: const Duration(seconds: 60),
      );
      expect(editableWith(pageText), findsOneWidget);
      final id = (await core(tester, (v) => v.listChildren(null)))
          .firstWhere((p) => p.title == pageTitle)
          .id;

      var text = pageText;
      final wallMs = <double>[];
      final notifications = <double>[];
      final bridgeCalls = <double>[];
      final frameMs = <double>[];
      final framesPerEdit = <double>[];

      for (var n = 0; n < edits; n++) {
        final line = editableWith(text).last;
        editState(tester, line).widget.focusNode.requestFocus();
        await tester.pump();
        final next = '$text${String.fromCharCode(0x61 + n % 26)}';

        var changes = 0;
        void changed() => changes++;
        var calls = 0;
        var lastInFlight = Vault.inFlight.value;
        void counted() {
          final now = Vault.inFlight.value;
          if (now > lastInFlight) calls += now - lastInFlight;
          lastInFlight = now;
        }

        db.app.pages.addListener(changed);
        Vault.inFlight.addListener(counted);
        final wall = Stopwatch()..start();
        List<double> frames;
        try {
          await tester.enterText(line, next);
          frames = await settleTimed(tester);
        } finally {
          wall.stop();
          db.app.pages.removeListener(changed);
          Vault.inFlight.removeListener(counted);
        }
        text = next;

        wallMs.add(wall.elapsedMicroseconds / 1000);
        notifications.add(changes.toDouble());
        bridgeCalls.add(calls.toDouble());
        frameMs.addAll(frames);
        framesPerEdit.add(frames.length.toDouble());
      }

      final page = await core(tester, (v) => v.loadPage(id));
      expect(page.blocks.first.text, text, reason: 'the edits must persist');

      // ignore: avoid_print
      print(
        'EDIT_PERF context rows=${db.app.pages.count} '
        'docs=${db.app.pages.docCount} edits=$edits',
      );
      // ignore: avoid_print
      print(summary('wall_ms', wallMs, 'ms'));
      // ignore: avoid_print
      print(summary('store_notifications', notifications, ''));
      // ignore: avoid_print
      print(summary('bridge_calls', bridgeCalls, ''));
      // ignore: avoid_print
      print(summary('frame_ms', frameMs, 'ms'));
      // ignore: avoid_print
      print(summary('frames_per_edit', framesPerEdit, ''));
    },
    timeout: const Timeout(Duration(minutes: 10)),
  );
}
