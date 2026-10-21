// Opt-in native-bridge UI trial of the filtered real BubblePlay replay.
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/ui/decision_line.dart';
import 'package:notes_app/ui/anchored_list.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen;
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);
  final source = Platform.environment['NOTES_LIVE_REPLAY_SOURCE'];
  final reportDir = Platform.environment['NOTES_LIVE_REPLAY_REPORT_DIR'];
  appTest(
    'filtered real replay has names and gists, clean cards and a full strip',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final replay = await real(
        tester,
        () => store.startReplay(source!, delayMs: 1),
      );
      final clock = Stopwatch()..start();
      while (!replay.finished &&
          replay.error == null &&
          clock.elapsed < const Duration(minutes: 3)) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 60)),
        );
        await tester.pump(const Duration(milliseconds: 60));
        expect(tester.takeException(), isNull);
      }
      expect(replay.error, isNull);
      expect(replay.finished, isTrue);
      expect(
        replay.userTexts.length,
        36,
        reason: 'four injected users are omitted',
      );
      expect(replay.sent, 36);
      await settleUntil(
        tester,
        () =>
            store.outline.chapters.length == 3 &&
            store.outline.chapters.expand((c) => c.sections).length == 8,
      );
      final chapters = store.outline.chapters;
      for (final chapter in chapters) {
        expect(chapter.decisions, isEmpty);
        for (final section in chapter.sections) {
          expect(section.decisions, isEmpty);
        }
      }
      final users = store.state!.messages
          .where((m) => m.role == 'user')
          .toList();
      expect(users.length, 36);
      expect(
        users.any(
          (m) =>
              m.text.trimLeft().startsWith('# AGENTS.md instructions') ||
              m.text.toLowerCase().contains('<instructions>'),
        ),
        isFalse,
      );
      expect(store.state!.runs.length, 36);
      expect(
        store.state!.runs.every(
          (r) => r.model == 'replay/replay' && r.status == 'completed',
        ),
        isTrue,
      );
      expect(find.byKey(const ValueKey('agent-session-project')), findsNothing);
      expect(find.text('Contents ▾'), findsOneWidget);

      Future<void> jump(String id) async {
        await tapAndSettle(tester, find.byKey(const ValueKey('outline-where')));
        await tapAndSettle(
          tester,
          find.byKey(ValueKey('outline-contents-chapter-$id')),
        );
      }

      Future<void> capture(String name) async {
        if (reportDir == null) return;
        final boundary = tester.renderObject<RenderRepaintBoundary>(
          find.byType(RepaintBoundary).first,
        );
        await real(tester, () async {
          final image = await boundary.toImage();
          try {
            final bytes = await image.toByteData(
              format: ui.ImageByteFormat.png,
            );
            await File('$reportDir/$name.png')
                .writeAsBytes(bytes!.buffer.asUint8List());
          } finally {
            image.dispose();
          }
        });
      }

      tester
          .widget<AnchoredList>(find.byType(AnchoredList))
          .controller
          .jumpToEnd();
      await settle(tester);
      await jump(chapters.first.id);
      await tapAndSettle(
        tester,
        find.byKey(ValueKey('outline-card-show-${chapters.first.id}')),
      );
      final card = tester.widget<OutlineChapterCard>(
        find.byKey(ValueKey('outline-chapter-${chapters.first.id}')),
      );
      expect(card.messagesShown, isFalse);
      expect(card.chapter.title, 'BubblePlay headless cursor');
      expect(card.chapter.gist, contains('Mouse Keys workaround hardened'));
      expect(find.byType(DecisionLine), findsNothing);
      await capture('polish-closed-chapter');
      await jump(chapters.last.id);
      expect(find.byType(OutlineChapterHeading), findsWidgets);
      expect(find.byType(DecisionLine), findsNothing);
      await capture('polish-open-chapter');
      if (reportDir != null) {
        await real(
          tester,
          () => File('$reportDir/polish-observations.json').writeAsString(
            const JsonEncoder.withIndent('  ').convert({
              'host': 'Flutter widget host with native Windows bridge',
              'elapsed_ms': clock.elapsedMilliseconds,
              'normal_runs': store.state!.runs.length,
              'native_messages': store.state!.messages.length,
              'chapters': chapters
                  .map(
                    (c) => {
                      'raw_title': c.title,
                      'name': outlineNameGist(c.title, c.summary).name,
                      'gist': outlineNameGist(c.title, c.summary).gist,
                      'sections': c.sections.length,
                      'ongoing': c.ongoing,
                      'decisions': c.decisions.length,
                    },
                  )
                  .toList(),
            }),
          ),
        );
      }
      await finishAsync(tester, store.close());
      expect(tester.takeException(), isNull);
    },
    skip: source == null || !devReplayEnabled,
    timeout: const Timeout(Duration(minutes: 4)),
  );
}
