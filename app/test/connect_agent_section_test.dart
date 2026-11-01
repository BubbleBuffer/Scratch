// Settings > Connectors > Apps that use your vault > Quick setup: the section
// shows the command for Claude Code and for Codex, Copy puts the Codex command
// on the clipboard, and phones leave it out. Nothing here calls a hub: the
// agent tokens section gets no client (AgentTokensSection.debugClientFor).

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/ai/agent_tokens_section.dart';

import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

String textOf(WidgetTester t, String k) => t.widget<Text>(key(k)).data!;

/// Opens Settings > Connectors with "Apps that use your vault" open.
Future<void> openAppsPanel(WidgetTester t, {bool mobile = false}) async {
  if (mobile) {
    await openMobileSettings(t);
  } else {
    await tapAndSettle(t, key('sidebar-settings'));
  }
  await tapAndSettle(t, key('settings-nav-connectors'));
  await t.ensureVisible(key('connectors-apps'));
  await tapAndSettle(t, key('connectors-apps'));
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
    await openAppsPanel(t);
    await showSection(t);

    expect(find.text('Quick setup'), findsOneWidget);
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
    await openAppsPanel(t);
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
    await openAppsPanel(t, mobile: true);

    expect(find.byKey(const ValueKey('connect-agent'), skipOffstage: false),
        findsNothing);
    expect(find.text('Access tokens', skipOffstage: false), findsOneWidget);
  });
}
