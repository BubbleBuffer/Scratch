// Opt-in real-source UI trial. This uses the widget host and native bridge;
// it is separate from a manual run of the packaged Windows application.
import 'dart:convert';
import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:agent_kit/agent_kit.dart'
    show AnchoredList, OutlineChapterCard, OutlineChapterHeading;
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);
  final source = Platform.environment['NOTES_LIVE_REPLAY_SOURCE'];
  final reportDir = Platform.environment['NOTES_LIVE_REPLAY_REPORT_DIR'];
  appTest(
    'real BubblePlay live replay uses normal runs and anchored live outline',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final playback = await real(
        tester,
        () => store.startReplay(source!, delayMs: 1),
      );
      var boundaryPaused = false;
      var boundaryChecked = false;
      // Hold the reader at the end of chapter one while its closure arrives.
      playback.addListener(() {
        if (playback.sent == 10 && playback.active && !boundaryPaused) {
          boundaryPaused = true;
          unawaited(playback.togglePause());
        }
      });
      final observations = <Map<String, Object?>>[];
      final clock = Stopwatch()..start();
      String previous = '';
      var sawDeferredFold = false;
      var sawOngoing = false;
      var measuredStability = false;
      while (!playback.finished &&
          playback.error == null &&
          clock.elapsed < const Duration(minutes: 4)) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 80)),
        );
        await tester.pump(const Duration(milliseconds: 80));
        final chapters = store.outline.chapters;
        if (chapters.isEmpty) continue;
        if (boundaryPaused &&
            playback.paused &&
            !boundaryChecked &&
            !chapters.first.ongoing) {
          await settle(tester);
          await tapAndSettle(
            tester,
            find.byKey(const ValueKey('outline-where')),
          );
          await tapAndSettle(
            tester,
            find.byKey(
              ValueKey('outline-contents-chapter-${chapters.first.id}'),
            ),
          );
          final heading = tester.widget<OutlineChapterHeading>(
            find.byKey(ValueKey('outline-chapter-${chapters.first.id}')),
          );
          expect(
            heading.closed,
            isTrue,
            reason: 'closing must not fold the chapter under the bottom reader',
          );
          sawDeferredFold = true;
          boundaryChecked = true;
          observations.add({
            'deferred_close_verified': true,
            'users_sent': playback.sent,
          });
          final list = tester.widget<AnchoredList>(find.byType(AnchoredList));
          list.controller.jumpToEnd();
          await settle(tester);
          await real(tester, playback.togglePause);
        }
        sawOngoing |= chapters.any((c) => c.ongoing);
        sawDeferredFold |= tester
            .widgetList<OutlineChapterHeading>(
              find.byType(OutlineChapterHeading),
            )
            .any((w) => w.closed);
        final signature =
            '${chapters.map((c) => '${c.id}:${c.sections.length}:${c.ongoing}').join(',')}:${store.state?.session.title}';
        if (signature != previous) {
          observations.add({
            'elapsed_ms': clock.elapsedMilliseconds,
            'users_sent': playback.sent,
            'messages': store.state?.messages.length,
            'title': store.state?.session.title,
            'chapters': [
              for (final c in chapters)
                {
                  'title': c.title,
                  'sections': c.sections.length,
                  'ongoing': c.ongoing,
                },
            ],
            'closed_heading_visible': tester
                .widgetList<OutlineChapterHeading>(
                  find.byType(OutlineChapterHeading),
                )
                .any((w) => w.closed),
          });
          previous = signature;
        }
        // Stay inside a built message, pause input, and verify an outline poll
        // leaves that message where it was. The normal reader is near the end.
        if (!measuredStability && chapters.first.sections.isNotEmpty) {
          await real(tester, playback.togglePause);
          await settle(tester);
          final anchors = find.byWidgetPredicate(
            (w) =>
                w.key is ValueKey<String> &&
                (w.key! as ValueKey<String>).value.startsWith('agent-msg-'),
          );
          if (anchors.evaluate().isNotEmpty) {
            final key = anchors.evaluate().last.widget.key!;
            final before = tester.getTopLeft(find.byKey(key)).dy;
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 650)),
            );
            await tester.pump(const Duration(milliseconds: 650));
            final after = tester.getTopLeft(find.byKey(key)).dy;
            expect(after, closeTo(before, 1));
            observations.add({
              'paused_anchor_y_before': before,
              'paused_anchor_y_after': after,
            });
            measuredStability = true;
          }
          await real(tester, playback.togglePause);
        }
        expect(tester.takeException(), isNull);
      }
      expect(playback.error, isNull);
      expect(playback.finished, isTrue);
      await settleUntil(
        tester,
        () =>
            store.outline.chapters.length == 3 &&
            store.outline.chapters.expand((c) => c.sections).length == 8,
      );
      expect(playback.sent, 40);
      expect(sawOngoing, isTrue);
      expect(measuredStability, isTrue);
      expect(boundaryChecked, isTrue);
      expect(
        store.state!.runs.every(
          (r) => r.model == 'replay/replay' && r.status == 'completed',
        ),
        isTrue,
      );
      expect(store.state!.session.title, store.outline.chapters.last.title);
      final ids = store.state!.messages.map((m) => m.id).toSet();
      for (final chapter in store.outline.chapters) {
        expect(
          ids,
          containsAll([chapter.firstMessageId, chapter.lastMessageId]),
        );
        for (final section in chapter.sections) {
          expect(
            ids,
            containsAll([section.firstMessageId, section.lastMessageId]),
          );
        }
      }
      final list = tester.widget<AnchoredList>(find.byType(AnchoredList));
      list.controller.jumpToEnd();
      await settle(tester, min: const Duration(milliseconds: 400));
      await tapAndSettle(tester, find.byKey(const ValueKey('outline-where')));
      final first = store.outline.chapters.first;
      await tapAndSettle(
        tester,
        find.byKey(ValueKey('outline-contents-chapter-${first.id}')),
      );
      final card = tester.widget<OutlineChapterCard>(
        find.byKey(ValueKey('outline-chapter-${first.id}')),
      );
      expect(card.messagesShown, isTrue);
      if (reportDir != null) {
        final dir = Directory(reportDir)..createSync(recursive: true);
        await real(
          tester,
          () => File('${dir.path}/observations.json').writeAsString(
            const JsonEncoder.withIndent('  ').convert({
              'host': 'Flutter widget test with the native Windows bridge',
              'elapsed_ms': clock.elapsedMilliseconds,
              'completed_users': playback.sent,
              'native_messages': store.state!.messages.length,
              'saw_deferred_fold': sawDeferredFold,
              'observations': observations,
            }),
          ),
        );
        final boundary = tester.renderObject<RenderRepaintBoundary>(
          find.byType(RepaintBoundary).first,
        );
        await real(tester, () async {
          final image = await boundary.toImage();
          try {
            final bytes = await image.toByteData(
              format: ui.ImageByteFormat.png,
            );
            await File('${dir.path}/first-chapter.png')
                .writeAsBytes(bytes!.buffer.asUint8List());
          } finally {
            image.dispose();
          }
        });
      }
      debugPrint(
        'Replay trial: closing the native store after ${playback.sent} users.',
      );
      await finishAsync(tester, store.close());
      debugPrint('Replay trial: native store closed.');
      expect(tester.takeException(), isNull);
    },
    skip: source == null || !devReplayEnabled,
    timeout: const Timeout(Duration(minutes: 5)),
  );
}
