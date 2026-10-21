// Golden screenshots of settings, pairing, the sync indicator and the
// conflict view, at 1440x900 and 375x812, light and dark, on the real core.
// Regenerate with: flutter test --update-goldens test/sync_golden_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/sync/pairing_codec.dart';
import 'package:notes_app/features/sync/sync_indicator.dart';
import 'package:notes_app/features/sync/sync_settings.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/state/sync_store.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'sync_helpers.dart';

const _token = '3f9a1c07d25e48b6a0f1e9c3b7d2a54e8c61f0b39d7a2e5c4b8f06a1d3e9c72b';

Future<void> shot(String name) async {
  await expectLater(find.byType(Directionality).first, matchesGoldenFile('../screenshots/$name.png'));
}

/// Stable addresses, and the hub shown as serving on every interface
/// (tests never bind 0.0.0.0).
void _fakeHub(SyncStore sync) {
  debugHubAddresses = const [
    HubAddress('100.101.102.103', AddressKind.tailscale, 'Tailscale'),
    HubAddress('192.168.1.20', AddressKind.lan, 'Ethernet'),
  ];
  sync.hubAddress = '0.0.0.0:47100';
  sync.notifyListeners();
}

class _States extends StatelessWidget {
  const _States();
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    return ColoredBox(
      color: c.bgPrimary,
      child: Padding(
        padding: EdgeInsets.fromLTRB(m ? Layout.gutter : 60, m ? 60 : 48, m ? Layout.gutter : 60, 0),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text('Sync indicator', style: ts(context.type.h3, c.textPrimary)),
          const SizedBox(height: 16),
          for (final p in [SyncPhase.synced, SyncPhase.syncing, SyncPhase.offline, SyncPhase.error]) ...[
            Container(
              height: Layout.topbarHeight,
              decoration: BoxDecoration(border: Border(bottom: BorderSide(color: c.borderSubtle))),
              child: Row(children: [
                SizedBox(width: m ? 90 : 120, child: Text(p.name, style: ts(context.type.ui, c.textSecondary, weight: 400))),
                SyncIndicatorView(phase: p, showLabel: !m, onTap: (_) {}),
                if (!m) ...[const SizedBox(width: 24), SyncIndicatorView(phase: p, showLabel: false, onTap: (_) {})],
              ]),
            ),
          ],
        ]),
      ),
    );
  }
}

void main() {
  setUpAll(initCore);
  tearDown(() => debugHubAddresses = null);

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      appTest('settings $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        if (mobile) {
          await openMobileSettings(tester);
          await shot('$tag-settings');
          await tapAndSettle(tester, key('settings-nav-sync'));
          await shot('$tag-settings-sync');
        } else {
          _fakeHub(tv.app.sync);
          await real(tester, () => tv.app.sync.pair('Pixel 8'));
          await real(tester, () => tv.app.sync.pair('Work laptop'));
          await tapAndSettle(tester, key('sidebar-settings'));
          await tapAndSettle(tester, key('settings-nav-sync'));
          await settle(tester);
          await shot('$tag-settings-sync');
          await tapAndSettle(tester, key('settings-nav-devices'));
          await shot('$tag-settings-devices');
        }
      });

      appTest('pairing $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        if (mobile) {
          await openMobileSettings(tester);
          await tapAndSettle(tester, key('settings-nav-sync'));
          await enterField(tester, 'connect-hub-field', const Pairing('http://100.101.102.103:47100', _token).encode());
          await shot('$tag-pairing');
        } else {
          _fakeHub(tv.app.sync);
          await tapAndSettle(tester, key('sidebar-settings'));
          await tapAndSettle(tester, key('settings-nav-devices'));
          showAddDevice(tester.element(key('devices-add')), token: _token);
          await settle(tester);
          await shot('$tag-pairing');
        }
      });

      appTest('sync indicator $tag', (tester) async {
        await pumpApp(tester, mobile: mobile, dark: dark);
        Navigator.of(tester.element(find.byType(HomeScreen))).push(PageRouteBuilder<void>(pageBuilder: (_, _, _) => const _States()));
        await settle(tester);
        await shot('$tag-sync-states');
      });

      appTest('sync error $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          other.hub = await real(tester, () => NotesHub.newInstance(vault: other.vault.raw));
          // A free port: a fixed one may still be held by an earlier test.
          final bound = await real(tester, () => other.hub!.start(addr: '127.0.0.1:0'));
          await real(tester, () => tv.app.sync.connect('http://$bound', 'ab' * 32));
          await settleUntil(tester, () => tv.app.sync.phase.$1 == SyncPhase.error);
          expect(tv.app.sync.phase.$1, SyncPhase.error);
          // The port differs per run: show a fixed one.
          tv.app.sync.hubUrl = 'http://127.0.0.1:47932';
          await tapAndSettle(tester, key('sync-indicator'));
          await shot('$tag-sync-error');
        } finally {
          await real(tester, () => tv.app.sync.disconnect());
          await other.close(tester);
        }
      });

      appTest('conflict $tag', (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        final other = await openOtherDevice(tester);
        try {
          await pairOther(tester, other);
          await makeConflict(tester, other, 'Weekly review',
              base: 'Review the week', here: 'Review the week on Monday', there: 'Friday: review the week');
          final weeklyReview = find.text('Weekly review').last;
          await tester.ensureVisible(weeklyReview);
          await settle(tester);
          await tapAndSettle(tester, weeklyReview);
          await shot('$tag-conflict-banner');
          await tapAndSettle(tester, key('conflict-review'));
          await settleUntil(tester, () => find.text('Friday: review the week').evaluate().isNotEmpty);
          await settle(tester);
          await shot('$tag-conflict');
        } finally {
          await real(tester, () => tv.app.sync.stopHub());
          await other.close(tester);
        }
      });
    }
  }
}
