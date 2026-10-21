// Native replay provider -> normal send/run/events -> outline polling -> chat.
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/core/file_dialogs.dart';
import 'package:notes_app/registry/command_registry.dart';
import 'package:notes_app/shell/commands.dart';
import 'package:notes_app/ui/anchored_list.dart';

import 'agent_test.dart' show openAgentStore, openAgentScreen, pumpUntil;
import 'harness.dart';
import 'sync_helpers.dart';

Future<void> stopReplay(WidgetTester tester, ReplayPlayback playback) async {
  late Future<void> stopped;
  // Keep the native cancellation polling on the real clock, while allowing
  // the command's already-started fake-zone continuations to complete.
  await tester.runAsync(() async {
    stopped = playback.stop();
  });
  await finishAsync(tester, stopped);
}

void main() {
  setUpAll(initCore);

  test('the replay command follows the dev flag', () {
    final registry = CommandRegistry();
    registerShellCommands(registry);
    expect(registry.of('agent.replay.live') != null, devReplayEnabled);
  });

  appTest('the replay command picks a file and a speed before starting', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await openAgentScreen(tester);
    final file = File('test/fixtures/chat_live_replay.sqlite').absolute;
    dialogs.files = [
      PickedFile('replay.sqlite', path: file.path, read: file.readAsBytes),
    ];
    final registry = CommandRegistry();
    registerShellCommands(registry);
    expect(
      registry.run('agent.replay.live', tester.element(find.byType(AgentChat))),
      isTrue,
    );
    await settle(tester);
    expect(dialogs.calls, contains('files'));
    expect(find.text('Delay in milliseconds'), findsOneWidget);
    await tester.enterText(
      find.descendant(
        of: find.byKey(const ValueKey('replay-delay')),
        matching: find.byType(EditableText),
      ),
      '5',
    );
    await tapAndSettle(tester, find.text('Start replay'));
    await pumpUntil(tester, () => store.replay != null);
    expect(store.replay!.delay, const Duration(milliseconds: 5));
    await stopReplay(tester, store.replay!);
  }, skip: !devReplayEnabled);

  appTest(
    'live replay sections arrive through native runs, chapters fold, and titles advance',
    (tester) async {
      expect(devReplayEnabled, isTrue, reason: 'Run with NOTES_DEV_REPLAY=1.');
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await openAgentScreen(tester);
      final playback = await real(
        tester,
        () => store.startReplay(
          File('test/fixtures/chat_live_replay.sqlite').absolute.path,
          delayMs: 100,
        ),
      );
      await pumpUntil(
        tester,
        () =>
            store.outline.chapters.isNotEmpty &&
            store.outline.chapters.first.sections.isNotEmpty &&
            store.state?.session.title == store.outline.chapters.first.title,
        diagnostics: () => '${playback.error}; ${store.error}',
      );
      expect(
        store.outline.chapters.first.sections.first.title,
        'First section',
      );
      expect(store.state!.session.title, store.outline.chapters.first.title);
      expect(playback.finished, isFalse);

      await real(tester, playback.togglePause);
      final count = store.state!.messages.length;
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 250)),
      );
      await tester.pump(const Duration(milliseconds: 500));
      // One already-settling normal tool result may arrive on the pause boundary.
      expect(store.state!.messages.length, lessThanOrEqualTo(count + 2));
      expect(find.text('Resume'), findsOneWidget);
      await real(tester, playback.togglePause);
      await pumpUntil(
        tester,
        () => playback.finished || playback.error != null,
        diagnostics: () => '${playback.error}; ${playback.sent}',
      );
      expect(playback.error, isNull);
      await pumpUntil(
        tester,
        () =>
            store.outline.chapters.length == 2 &&
            store.outline.chapters.last.sections.length == 1 &&
            store.state!.session.title == store.outline.chapters.last.title,
      );
      expect(store.outline.chapters.first.ongoing, isFalse);
      expect(store.outline.chapters.first.sections.length, 2);
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
      await settle(tester);
      await tapAndSettle(tester, find.byKey(const ValueKey('outline-where')));
      await tapAndSettle(
        tester,
        find.byKey(
          ValueKey(
            'outline-contents-chapter-${store.outline.chapters.first.id}',
          ),
        ),
      );
      expect(find.byType(OutlineChapterCard), findsWidgets);
      expect(
        store.state!.runs.every(
          (r) => r.model == 'replay/replay' && r.status == 'completed',
        ),
        isTrue,
      );
      expect(tester.takeException(), isNull);
    },
    skip: !devReplayEnabled,
  );

  appTest('stopping a paused replay cancels its normal native run', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    final playback = await real(
      tester,
      () => store.startReplay(
        File('test/fixtures/chat_live_replay.sqlite').absolute.path,
        delayMs: 100,
      ),
    );
    await real(tester, playback.togglePause);
    await stopReplay(tester, playback);
    expect(playback.stopped, isTrue);
    expect(playback.finished, isFalse);
    final agent = (await real(tester, store.ensureOpen))!;
    final state = await real(
      tester,
      () => agent.sessionState(sessionId: playback.sessionId),
    );
    expect(
      state.runs.any((r) => r.status == 'running' || r.status == 'pending'),
      isFalse,
    );
    expect(tester.takeException(), isNull);
  }, skip: !devReplayEnabled);
}
