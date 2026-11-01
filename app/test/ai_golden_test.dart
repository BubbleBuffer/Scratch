// Golden screenshots of AI: Settings > Agent with tasks in each state and the
// Ask AI box, at 1440x900 and 375x812, light and dark, on the real core.
// Regenerate with: flutter test --update-goldens test/ai_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/ai_settings.dart';
import 'package:notes_app/features/ai/ai_store.dart';
import 'package:notes_app/features/ai/ask_ai.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/src/rust/api/ai.dart';

import 'ai_test.dart' show aiStore, closeAi;
import 'harness.dart';
import 'sync_helpers.dart';

Future<void> shot(String name) async {
  await expectLater(find.byType(Directionality).first, matchesGoldenFile('../screenshots/$name.png'));
}

/// Three tasks: done, failed and queued (written directly, no agent).
Future<void> _tasks(WidgetTester tester, AiStore store) async {
  final v = current.vault.raw;
  final page = current.pages['Garden plan']!;
  final done = await real(tester,
      () => aiCreateTask(vault: v, prompt: 'Summarize the garden plan', page: page, after: null, runner: ''));
  await real(tester, () => aiClaimNext(vault: v));
  await real(tester, () => aiFinishTask(vault: v, id: done.id, ok: true, text: 'Tomatoes and basil by the fence.'));
  final failed = await real(tester,
      () => aiCreateTask(vault: v, prompt: 'Draft a weekly review', page: page, after: null, runner: ''));
  await real(tester, () => aiClaimNext(vault: v));
  await real(tester,
      () => aiFinishTask(vault: v, id: failed.id, ok: false, text: 'the agent exited with code 1: not logged in'));
  await real(tester, store.refresh);
  await settle(tester);
}

void main() {
  setUpAll(initCore);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      appTest('settings ai $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        // Seed terminal task states before enabling the runner; the feed can
        // otherwise claim a fixture task between its create and finish calls.
        final store = await aiStore(tester,
            // The agent off: its model list comes from this computer.
            settings: AiSettings()..agentEnabled = false);
        try {
          await _tasks(tester, store);
          if (mobile) {
            await openMobileSettings(tester);
          } else {
            await tapAndSettle(tester, key('sidebar-settings'));
          }
          await tapAndSettle(tester, key('settings-nav-agent'));
          await shot('$tag-settings-ai');
        } finally {
          await closeAi(tester, store);
        }
      });

      appTest('ask ai $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        final store = await aiStore(tester, settings: AiSettings()..agentEnabled = false);
        try {
          showAskAi(tester.element(find.byType(HomeScreen)), pageId: current.pages['Garden plan']!);
          await settle(tester);
          await enterField(tester, 'ask-ai-field', 'Add a watering schedule');
          await shot('$tag-ask-ai');
          await tester.sendKeyEvent(LogicalKeyboardKey.escape);
          await settle(tester);
        } finally {
          await closeAi(tester, store);
        }
      });
    }
  }
}
