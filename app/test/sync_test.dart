// Sync and settings on the real core: the app's hub in-process on
// 127.0.0.1 with a second temporary vault as the other device.
// - pairing from the settings UI, edits both ways, unpairing;
// - a text conflict opened from settings and resolved inline on its note;
// - a phone-sized app connecting to a hub with a pairing code;
// - the theme setting.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/conflict_copy.dart';
import 'package:notes_app/features/sync/conflicts.dart';
import 'package:notes_app/features/sync/pairing_codec.dart';
import 'package:notes_app/src/rust/api/sync.dart' show redeemPairingCode;
import 'package:notes_app/state/sync_store.dart';

import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);
  tearDown(() {
    final s = AppSettings.memory;
    s.clientUrl = null;
    s.clientToken = null;
    s.theme = ThemePref.system;
  });

  appTest('pair a device from settings; edits sync both ways; unpair', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    // Discovery is deterministic; the transport below reaches this same hub
    // through loopback, since the test does not own a physical LAN address.
    debugHubAddresses = const [HubAddress('192.168.0.5', AddressKind.lan, 'test')];
    addTearDown(() => debugHubAddresses = null);
    try {
      final sync = tv.app.sync;
      await real(tester, () => sync.startHub('0.0.0.0:0'));
      await settle(tester);

      await tapAndSettle(tester, key('sidebar-settings'));
      // Devices are part of the "Sync and devices" page, below the sync part.
      await tapAndSettle(tester, key('settings-nav-sync'));
      expect(find.text('No devices paired yet'), findsOneWidget);
      await tester.ensureVisible(key('devices-add'));
      await tapAndSettle(tester, key('devices-add'));
      await enterField(tester, 'device-name-field', 'Pixel');
      await tester.tap(key('create-pairing'));
      await settleUntil(tester, () => key('pair-token').evaluate().isNotEmpty);

      // A one-time pairing code (the field keeps its old key).
      final pairCode = textOf(tester, 'pair-token');
      final url = textOf(tester, 'pair-url');
      final code = Pairing.parse(textOf(tester, 'pair-code'))!;
      expect(key('pairing-qr'), findsOneWidget);
      expect(url, 'http://192.168.0.5:${portOf(sync.hubAddress!)}');
      expect(code.url, url);
      expect(code.code, pairCode);
      expect(code.token, isEmpty);

      await tapAndSettle(tester, key('pair-done'));
      expect(key('device-Pixel'), findsOneWidget);

      // The other device's first sync brings every page.
      // The pairing URL names the fake LAN address; the other device reaches the
      // hub on loopback.
      final reachable = 'http://127.0.0.1:${portOf(sync.hubAddress!)}';
      final token = await real(
        tester,
        () => redeemPairingCode(vault: other.vault.raw, hubUrl: reachable, code: pairCode, trusted: const []),
      );
      other.client = NotesSync(vault: other.vault.raw, url: reachable, token: token);
      final first = await real(tester, () => other.client!.syncOnce());
      expect(first.full, isTrue);
      // Both desktop app opens publish their node record under hidden Nodes.
      // As before, the records count as pages; the Nodes parent does not.
      expect(await real(tester, () => other.vault.pageCount()), tv.pages.length + 2);

      // Close settings (Escape), then an edit there shows up here.
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(key('settings-nav-sync'), findsNothing);
      await real(tester, () => other.vault.renamePage(tv.pages['Budget']!, 'Budget 2027'));
      await real(tester, () => other.client!.syncOnce());
      await settleUntil(tester, () => find.text('Budget 2027').evaluate().isNotEmpty);
      expect(find.text('Budget 2027'), findsWidgets);

      // An edit typed here reaches the other device, and its edit to the
      // open page appears in the editor.
      await tapAndSettle(tester, find.text('Garden plan').last);
      final b = editableWith('Some text about garden plan.');
      await focusAt(tester, b, 'Some text about garden plan.'.length);
      await typeText(tester, ' Tomatoes');
      await settle(tester);
      await real(tester, () => tv.app.pageStore.lastSave);
      await real(tester, () => tv.vault.flush());

      final gid = tv.pages['Garden plan']!;
      final there = await real(tester, () => other.vault.loadPage(gid));
      await real(tester, () => other.vault.apply(gid, [
            BOp(
                op: OpKind.insert,
                id: newId(),
                after: there.blocks.last.id,
                kind: Kinds.paragraph,
                text: 'From the phone',
                marks: const [],
                checked: false),
          ]));
      await real(tester, () => other.client!.syncOnce());
      final synced = await real(tester, () => other.vault.loadPage(gid));
      expect(synced.blocks.map((b) => b.text), contains('Some text about garden plan. Tomatoes'));
      await settleUntil(tester, () => editableWith('From the phone').evaluate().isNotEmpty);
      expect(editableWith('From the phone'), findsOneWidget);
      expect(editableWith('Some text about garden plan. Tomatoes'), findsOneWidget);

      // Unpair: the device is listed as connected, then its token stops working.
      await tapAndSettle(tester, key('sidebar-settings'));
      await tapAndSettle(tester, key('settings-nav-sync'));
      await settleUntil(tester, () => sync.devices.any((d) => d.device != null));
      expect(sync.devices.single.device, other.vault.deviceId);
      await tester.ensureVisible(key('unpair-Pixel'));
      await tester.tap(key('unpair-Pixel'));
      await settleUntil(tester, () => sync.devices.isEmpty);
      expect(key('device-Pixel'), findsNothing);
      Object? error;
      await tester.runAsync(() async {
        try {
          await other.client!.syncOnce();
        } catch (e) {
          error = e;
        }
      });
      expect('$error', contains('401'));
    } finally {
      await other.close(tester);
    }
  });

  appTest('a conflict opens from settings onto its note; the other text resolves it inline', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final block = await makeConflict(tester, other, 'Weekly review');
      final page = tv.pages['Weekly review']!;

      // Settings lists the page; its row opens the note itself.
      await openConflictSettings(tester);
      expect(key('conflict-page-$page'), findsOneWidget);
      await tapAndSettle(tester, key('conflict-page-$page'));
      expect(key('settings-nav-conflicts'), findsNothing, reason: 'the row opens the note');
      expect(find.text('Weekly review'), findsWidgets);

      // The offered text is the side the note does not show (the later edit
      // is shown): read it from the conflict, not from fixed strings.
      final c = tv.app.sync.conflictsOf(page).single;
      final alt = conflictShownSide(c) == 0 ? 1 : 0;
      final offered = c.sides[alt].block!.text;
      final toast = c.sides[alt].local ? conflictUsedLocalLine : conflictUsedCloudLine;

      // Inline on the note: the other text sits beside the shown text.
      expect(find.text(offered), findsOneWidget);
      expect(find.text('Local'), findsWidgets);
      expect(find.text('Cloud · from Phone'), findsOneWidget, reason: 'the other side is labelled with its paired name');

      // One tap on the other text takes it.
      await tapAndSettle(tester, find.text(offered), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(tv.app.sync.conflicts, isEmpty);
      expect((await blockById(tester, page, block)).text, offered);
      expect(find.text(toast), findsOneWidget);

      // The resolution reaches the other device with its next sync.
      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
    } finally {
      await other.close(tester);
    }
  });

  appTest('Keep leaves a text conflict as the note shows it and dismisses it', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      await pairOther(tester, other);
      final block = await makeConflict(tester, other, 'Recipes', here: 'Soup on Sunday', there: 'Bread on Sunday');
      final page = tv.pages['Recipes']!;
      final before = (await blockById(tester, page, block)).text;
      final c = tv.app.sync.conflictsOf(page).single;
      final offered = c.sides[conflictShownSide(c) == 0 ? 1 : 0].block!.text;

      await tapAndSettle(tester, find.text('Recipes').last);
      await settleUntil(tester, () => key('keep-$block').evaluate().isNotEmpty);
      // Waits for the native keep, so the absent toast is checked after the work.
      await tapAndSettle(tester, key('keep-$block'), waitForBridge: true);
      await settleUntil(tester, () => tv.app.sync.conflicts.isEmpty);
      expect(tv.app.sync.conflicts, isEmpty);
      expect((await blockById(tester, page, block)).text, before, reason: 'Keep does not change the text');
      expect(find.text(offered), findsNothing, reason: 'the dismissed text is no longer offered');
      expect(key('conflict-toast'), findsNothing, reason: 'Keep on a block shows no toast');
      expect(find.text('Undo'), findsNothing);

      await real(tester, () => other.client!.syncOnce());
      expect(await real(tester, () => other.vault.run((v) => v.listConflicts(page: null))), isEmpty);
    } finally {
      await other.close(tester);
    }
  });

  appTest('phone: connect to a hub with a pairing code; sync runs in the background', (tester) async {
    final tv = await pumpApp(tester, mobile: true);
    final other = await openOtherDevice(tester);
    try {
      expect(SyncStore.canHost, isFalse);
      final hubPage = await real(tester, () => other.vault.createPage(title: 'Made on the desktop'));
      other.hub = await real(tester, () => NotesHub.newInstance(vault: other.vault.raw));
      final bound = await real(tester, () => other.hub!.start(addr: '127.0.0.1:0'));
      final pairCode = await real(tester, () => other.hub!.pair(label: 'Phone'));
      final code = Pairing.code('http://$bound', pairCode).encode();

      expect(key('sync-indicator'), findsNothing, reason: 'hidden while sync is off');
      await openMobileSettings(tester);
      await tapAndSettle(tester, key('settings-nav-sync'));
      expect(key('hub-toggle'), findsNothing, reason: 'phones do not host');
      await tapAndSettle(tester, key('connect-advanced'));
      await enterField(tester, 'connect-hub-field', code);
      expect(tester.widget<EditableText>(find.descendant(of: key('connect-token-field'), matching: find.byType(EditableText))).controller.text,
          isEmpty, reason: 'a code link fills only the hub; connecting redeems the code');
      await tester.ensureVisible(key('connect-button'));
      await tester.tap(key('connect-button'));
      final sync = tv.app.sync;
      await settleUntil(tester, () => sync.status.lastOkMs != null, timeout: const Duration(seconds: 20));
      expect(sync.phase.$1, SyncPhase.synced);
      expect(current.app.settings.clientUrl, 'http://$bound');
      expect(find.textContaining('Connected to http://$bound'), findsOneWidget);

      // Home lists the hub's page and shows the indicator.
      // Return from connection/settings, then close the tree before Home.
      await tapAndSettle(tester, key('back'));
      await tapAndSettle(tester, key('back'));
      await closeMobileTreeWithBack(tester);
      expect(key('capture-field'), findsOneWidget);
      expect(key('sync-indicator'), findsOneWidget);
      await expandAllPages(tester);
      await settleUntil(tester, () => find.text('Made on the desktop').evaluate().isNotEmpty);
      expect(find.text('Made on the desktop'), findsWidgets);
      // A change on the hub arrives without asking (the loop long-polls).
      await real(tester, () => other.vault.renamePage(hubPage, 'Renamed on the desktop'));
      await settleUntil(tester, () => find.text('Renamed on the desktop').evaluate().isNotEmpty,
          timeout: const Duration(seconds: 20));
      expect(find.text('Renamed on the desktop'), findsWidgets);

      // And a page made here reaches the hub.
      final mine = await core(tester, (v) => v.createPage(title: 'Made on the phone'));
      final sw = Stopwatch()..start();
      var arrived = false;
      while (!arrived && sw.elapsed < const Duration(seconds: 20)) {
        await settle(tester, min: Duration.zero);
        await tester.runAsync(() => Future<void>.delayed(Duration.zero));
        arrived = await real(tester, () async {
          try {
            return (await other.vault.loadPage(mine)).title == 'Made on the phone';
          } catch (_) {
            return false;
          }
        });
      }
      expect(arrived, isTrue);

      // Disconnect clears the saved connection.
      await openMobileSettings(tester);
      await tapAndSettle(tester, key('settings-nav-sync'));
      await tester.tap(key('disconnect-button'));
      await settleUntil(tester, () => !sync.connected && current.app.settings.clientUrl == null);
      expect(current.app.settings.clientUrl, isNull);
      expect(key('connect-button'), findsOneWidget);
    } finally {
      await real(tester, () => tv.app.sync.disconnect());
      await other.close(tester);
    }
  });

  appTest('a bad token shows as a sync error with its message', (tester) async {
    final tv = await pumpApp(tester, mobile: false);
    final other = await openOtherDevice(tester);
    try {
      other.hub = await real(tester, () => NotesHub.newInstance(vault: other.vault.raw));
      final bound = await real(tester, () => other.hub!.start(addr: '127.0.0.1:0'));
      await real(tester, () => tv.app.sync.connect('http://$bound', 'ab' * 32));
      await settleUntil(tester, () => tv.app.sync.phase.$1 == SyncPhase.error);
      expect(find.text('Sync error'), findsOneWidget);
      await tapAndSettle(tester, key('sync-indicator'));
      expect(textOf(tester, 'sync-error-message'), contains('401'));
    } finally {
      await real(tester, () => tv.app.sync.disconnect());
      await other.close(tester);
    }
  });

  appTest('the theme setting switches to dark', (tester) async {
    await pumpApp(tester, mobile: false);
    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('theme-dark'));
    expect(current.app.settings.theme, ThemePref.dark);
    expect(current.app.settings.theme.forceDark, isTrue);
  });
}
