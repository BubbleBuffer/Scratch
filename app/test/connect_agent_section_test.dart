// Settings > AI > Connect an agent: the section shows the command for Claude
// Code and for Codex, Copy puts the Codex command on the clipboard, and phones
// show the section too. Nothing here calls a hub: the agent tokens section
// gets no client (AgentTokensSection.debugClientFor).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/agent_tokens_section.dart';

import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

String textOf(WidgetTester t, String k) => t.widget<Text>(key(k)).data!;

Future<void> openAiSettings(WidgetTester t) async {
  await tapAndSettle(t, key('sidebar-settings'));
  await tapAndSettle(t, key('settings-nav-ai'));
  await real(t, () => Future<void>.delayed(const Duration(milliseconds: 20)));
  await settle(t);
}

/// Scrolls the section into view: Settings is longer than the test window.
Future<void> showSection(WidgetTester t) async {
  await t.ensureVisible(key('connect-agent'));
  await settle(t);
}

void main() {
  setUpAll(initCore);

  setUp(() => AgentTokensSection.debugClientFor = (_) => null);
  tearDown(() => AgentTokensSection.debugClientFor = null);

  appTest('desktop shows the Claude Code and Codex commands', (t) async {
    await pumpApp(t, mobile: false);
    await openAiSettings(t);
    await showSection(t);

    expect(find.text('Connect an agent'), findsOneWidget);
    expect(
      find.descendant(
        of: key('connect-agent-row-claude'),
        matching: find.text('Claude Code'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: key('connect-agent-row-codex'),
        matching: find.text('Codex'),
      ),
      findsOneWidget,
    );
    expect(textOf(t, 'connect-agent-claude'), 'notes setup claude');
    expect(textOf(t, 'connect-agent-codex'), 'notes setup codex');
    // The section sits above the agent tokens.
    expect(
      t.getTopLeft(key('connect-agent')).dy,
      lessThan(t.getTopLeft(key('agent-tokens')).dy),
    );
  });

  appTest('Copy for Codex puts the codex command on the clipboard', (
    t,
  ) async {
    final copied = <String>[];
    t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          copied.add((call.arguments as Map)['text'] as String);
        }
        return null;
      },
    );
    addTearDown(
      () => t.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        null,
      ),
    );
    await pumpApp(t, mobile: false);
    await openAiSettings(t);
    await showSection(t);

    final copy = find.descendant(
      of: key('connect-agent-row-codex'),
      matching: find.text('Copy'),
    );
    await t.ensureVisible(copy);
    await settle(t);
    await tapAndSettle(t, copy);
    expect(copied, ['notes setup codex']);
    // CopyableText shows "Copied" for two seconds; let that pass.
    await t.pump(const Duration(seconds: 3));
  });

  appTest('phones do not show the connect an agent section: its commands '
      'run in a desktop terminal', (t) async {
    await pumpApp(t, mobile: true);
    await openMobileSettings(t);
    await tapAndSettle(t, key('settings-nav-ai'));
    await real(t, () => Future<void>.delayed(const Duration(milliseconds: 20)));
    await settle(t);

    expect(find.byKey(const ValueKey('connect-agent'), skipOffstage: false),
        findsNothing);
    expect(find.text('Agent tokens', skipOffstage: false), findsOneWidget);
  });
}
