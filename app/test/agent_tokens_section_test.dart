// Settings > AI > Agent tokens: the hub's tokens are listed, a new one's
// secret is shown once with a Copy button, and Revoke removes one after a
// confirmation. The hub is an in-memory fake client
// (`AgentTokensSection.debugClientFor`); the HTTP side is covered by
// agent_token_client_test.dart and the hub's own tests.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/clock.dart';
import 'package:notes_app/features/ai/agent_token_client.dart';
import 'package:notes_app/features/ai/agent_tokens_section.dart';
import 'package:notes_app/features/settings/controls.dart';
import 'package:notes_app/ui/format.dart';

import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const _day = 24 * 60 * 60 * 1000;
// Fake secrets: obviously not real tokens.
const _secret = 'fake-secret-for-widget-test-0001';

class _FakeHub extends AgentTokenClient {
  _FakeHub()
    : super(hubUrl: 'https://hub.example', token: 'device', deviceId: 'dev');

  final tokens = <AgentTokenInfo>[];
  final created = <(String, String, int?)>[];
  final revoked = <String>[];
  int listCalls = 0;

  @override
  Future<AgentTokenList> list() async {
    listCalls++;
    return AgentTokenList(List.of(tokens), agentTokenExpiryChoices, 90);
  }

  @override
  Future<CreatedAgentToken> create({
    required String name,
    required String scope,
    required int? expiresDays,
  }) async {
    if (tokens.any((t) => t.name == name)) {
      throw const AgentTokenException(
        'A token with this name already exists.',
        status: 409,
      );
    }
    created.add((name, scope, expiresDays));
    final now = DateTime.now().millisecondsSinceEpoch;
    final info = AgentTokenInfo(
      name: name,
      scope: scope,
      created: now,
      expires: expiresDays == null ? null : now + expiresDays * _day,
    );
    tokens.add(info);
    return CreatedAgentToken(info, _secret);
  }

  @override
  Future<void> revoke(String name) async {
    revoked.add(name);
    tokens.removeWhere((t) => t.name == name);
  }
}

Future<void> openAiSettings(WidgetTester t) async {
  await tapAndSettle(t, key('sidebar-settings'));
  await tapAndSettle(t, key('settings-nav-ai'));
  await real(t, () => Future<void>.delayed(const Duration(milliseconds: 20)));
  await settle(t);
}

Future<void> showSection(WidgetTester t) async {
  await t.ensureVisible(key('agent-tokens'));
  await settle(t);
}

String textOf(WidgetTester t, String k) => t.widget<Text>(key(k)).data!;

/// Scrolls [k] into view first: Settings is longer than the test window.
Future<void> tapKey(WidgetTester t, String k) async {
  await t.ensureVisible(key(k));
  await settle(t);
  await tapAndSettle(t, key(k));
}

Future<void> typeInto(WidgetTester t, String k, String text) async {
  await t.ensureVisible(key(k));
  await settle(t);
  await t.enterText(key(k), text);
  await settle(t);
}

void main() {
  setUpAll(initCore);

  late _FakeHub hub;
  setUp(() {
    hub = _FakeHub();
    AgentTokensSection.debugClientFor = (_) => hub;
  });
  tearDown(() => AgentTokensSection.debugClientFor = null);

  appTest('lists the hub\'s tokens with access, last use and expiry', (
    t,
  ) async {
    // The app's clock, which relativeTime reads; pumpApp sets it.
    await pumpApp(t, mobile: false);
    final now = clockNow().millisecondsSinceEpoch;
    hub.tokens.addAll([
      AgentTokenInfo(
        name: 'laptop-claude',
        scope: 'write',
        created: now - 10 * _day,
        lastUsed: now - 5 * 60 * 1000,
        expires: now + 80 * _day,
      ),
      AgentTokenInfo(name: 'ci-reader', scope: 'read', created: now - _day),
      AgentTokenInfo(
        name: 'old-bot',
        scope: 'read',
        created: now - 400 * _day,
        expires: now - 35 * _day,
        expired: true,
      ),
    ]);
    await openAiSettings(t);
    await showSection(t);

    expect(key('agent-token-laptop-claude'), findsOneWidget);
    expect(find.text('laptop-claude'), findsOneWidget);
    final laptop = find.descendant(
      of: key('agent-token-laptop-claude'),
      matching: find.textContaining('Read and write'),
    );
    expect(laptop, findsOneWidget);
    final summary = t.widget<Text>(laptop).data!;
    // Relative to the app's clock (5m ago when it is the real one).
    expect(
      summary,
      contains('Last used ${relativeTime(now - 5 * 60 * 1000).toLowerCase()}'),
    );
    expect(summary, isNot(contains('Never used')));
    expect(summary, contains('Expires '));
    expect(
      find.descendant(
        of: key('agent-token-ci-reader'),
        matching: find.textContaining('Read only · Created'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: key('agent-token-ci-reader'),
        matching: find.textContaining('Never used · Never expires'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: key('agent-token-old-bot'),
        matching: find.textContaining('Expired '),
      ),
      findsOneWidget,
    );
    expect(key('agent-tokens-empty'), findsNothing);
  });

  appTest('creating a token shows its secret once, with Copy', (t) async {
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
    expect(key('agent-tokens-empty'), findsOneWidget);

    await tapKey(t, 'agent-token-new');
    expect(key('agent-token-form'), findsOneWidget);
    // A bad name is refused before the hub is asked.
    await typeInto(t, 'agent-token-name', 'Bad Name');
    await tapKey(t, 'agent-token-create');
    expect(hub.created, isEmpty);

    await typeInto(t, 'agent-token-name', 'laptop-codex');
    await tapKey(t, 'agent-token-create');
    await real(t, () => Future<void>.delayed(const Duration(milliseconds: 20)));
    await settle(t);
    // Defaults: read and write, 90 days.
    expect(hub.created, [('laptop-codex', 'write', 90)]);
    expect(key('agent-token-form'), findsNothing);
    expect(key('agent-token-created'), findsOneWidget);
    expect(textOf(t, 'agent-token-secret'), _secret);
    expect(key('agent-token-laptop-codex'), findsOneWidget);

    final copy = find.descendant(
      of: key('agent-token-created'),
      matching: find.text('Copy'),
    );
    await t.ensureVisible(copy);
    await settle(t);
    await tapAndSettle(t, copy);
    expect(copied, [_secret]);
    await t.pump(const Duration(seconds: 3));

    // Done hides it, and nothing shows it again.
    await tapKey(t, 'agent-token-secret-done');
    expect(key('agent-token-created'), findsNothing);
    expect(find.textContaining(_secret), findsNothing);
    expect(key('agent-token-laptop-codex'), findsOneWidget);
  });

  appTest('the create form offers the tool sets, Everything always', (
    t,
  ) async {
    await pumpApp(t, mobile: false);
    await openAiSettings(t);
    await showSection(t);
    await tapKey(t, 'agent-token-new');
    final picker = t.widget<SelectButton<String>>(key('agent-token-scope'));
    expect(picker.options, ['read', 'write', 'everything']);
    expect(picker.value, 'write');

    await tapKey(t, 'agent-token-scope');
    await tapAndSettle(t, find.text('Everything').last);
    await typeInto(t, 'agent-token-name', 'all-tools');
    await tapKey(t, 'agent-token-create');
    await real(t, () => Future<void>.delayed(const Duration(milliseconds: 20)));
    await settle(t);
    expect(hub.created, [('all-tools', 'everything', 90)]);
    expect(
      find.descendant(
        of: key('agent-token-all-tools'),
        matching: find.textContaining('Everything · Created'),
      ),
      findsOneWidget,
    );
  });

  appTest('a duplicate name shows the hub\'s error and keeps the form', (
    t,
  ) async {
    hub.tokens.add(
      AgentTokenInfo(name: 'taken', scope: 'read', created: 1),
    );
    await pumpApp(t, mobile: false);
    await openAiSettings(t);
    await showSection(t);
    await tapKey(t, 'agent-token-new');
    await typeInto(t, 'agent-token-name', 'taken');
    await tapKey(t, 'agent-token-create');
    await settle(t);
    expect(key('agent-token-form'), findsOneWidget);
    expect(key('agent-token-created'), findsNothing);
  });

  appTest('revoke asks first, then removes the token', (t) async {
    hub.tokens.addAll([
      AgentTokenInfo(name: 'keep-me', scope: 'read', created: 1),
      AgentTokenInfo(name: 'drop-me', scope: 'write', created: 2),
    ]);
    await pumpApp(t, mobile: false);
    await openAiSettings(t);
    await showSection(t);

    // Cancelling the confirmation revokes nothing.
    await tapKey(t, 'agent-token-revoke-drop-me');
    expect(key('confirm-ok'), findsOneWidget);
    await t.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(t);
    expect(hub.revoked, isEmpty);
    expect(key('agent-token-drop-me'), findsOneWidget);

    await tapKey(t, 'agent-token-revoke-drop-me');
    await tapAndSettle(t, key('confirm-ok'));
    await real(t, () => Future<void>.delayed(const Duration(milliseconds: 20)));
    await settle(t);
    expect(hub.revoked, ['drop-me']);
    expect(key('agent-token-drop-me'), findsNothing);
    expect(key('agent-token-keep-me'), findsOneWidget);
  });

  appTest('a device not signed in to a hub is told to sign in', (t) async {
    AgentTokensSection.debugClientFor = (_) => null;
    await pumpApp(t, mobile: false);
    await openAiSettings(t);
    await showSection(t);
    expect(key('agent-tokens-signed-out'), findsOneWidget);
    expect(key('agent-token-new'), findsNothing);
  });

  appTest('phones manage tokens too', (t) async {
    hub.tokens.add(AgentTokenInfo(name: 'phone-made', scope: 'read', created: 1));
    await pumpApp(t, mobile: true);
    await openMobileSettings(t);
    await tapAndSettle(t, key('settings-nav-ai'));
    await real(t, () => Future<void>.delayed(const Duration(milliseconds: 20)));
    await settle(t);
    await showSection(t);
    expect(key('agent-token-phone-made'), findsOneWidget);
    expect(key('agent-token-new'), findsOneWidget);
  });
}
