// Screenshots of the untrusted-certificate states (indicator details, Sync
// settings, the certificate view, the trusted list) at 1440x900 and
// 375x812, light and dark, for review. They are written to
// build/cert-shots/ (not committed), so these run only when updating:
//   flutter test --update-goldens test/cert_shots_test.dart

@Tags(['golden'])
library;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/cert_trust.dart';
import 'package:notes_app/features/sync/certificate_view.dart';
import 'package:notes_app/features/sync/sync_control.dart';

import 'cert_helpers.dart';
import 'harness.dart';
import 'sync_helpers.dart';

Future<void> shot(String name) async {
  await expectLater(find.byType(Directionality).first, matchesGoldenFile('../build/cert-shots/$name.png'));
}

void main() {
  setUpAll(initCore);
  tearDown(() {
    FakeHubs.uninstall();
    final s = AppSettings.memory;
    s.clientUrl = null;
    s.clientToken = null;
    s.trustedCertificates = TrustedCertificates();
  });
  final skip = !autoUpdateGoldenFiles;

  for (final mobile in [false, true]) {
    for (final dark in [false, true]) {
      final tag = '${mobile ? 'mobile' : 'desktop'}-${dark ? 'dark' : 'light'}';

      appTest('certificate error and view $tag', skip: skip, (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        FakeHubs().install();
        final s = AppSettings.read(homeContext(tester));
        await connectToHub(tv.app.sync, s, hubUrl, 'ab' * 32);
        await settle(tester);

        await tapAndSettle(tester, key('sync-indicator'));
        await shot('$tag-cert-error');
        await tapAndSettle(tester, key('cert-view'));
        await shot('$tag-cert-view');
        await tester.ensureVisible(key('cert-chain-row-1'));
        await tapAndSettle(tester, key('cert-chain-row-1'));
        await shot('$tag-cert-view-chain');
        await tester.ensureVisible(key('cert-view-trust'));
        await tapAndSettle(tester, key('cert-view-trust'));
        await shot('$tag-cert-confirm');
        await tapAndSettle(tester, key('confirm-cancel'));
        await tv.app.sync.disconnect();
      });

      appTest('certificate settings $tag', skip: skip, (tester) async {
        final tv = await pumpApp(tester, mobile: mobile, dark: dark);
        final hubs = FakeHubs()..install();
        final s = AppSettings.read(homeContext(tester));
        // A pin for another hub, and this hub's certificate untrusted.
        s.update((s) => s.trustedCertificates.add(
            'https://10.0.0.2:47100',
            PinnedCertificate.of([certDetailsOf(caCert)], trustedAtMs: validFrom)));
        await connectToHub(tv.app.sync, s, hubUrl, 'ab' * 32);
        await settle(tester);
        if (mobile) {
          await openMobileSettings(tester);
        } else {
          await tapAndSettle(tester, key('sidebar-settings'));
        }
        await tapAndSettle(tester, key('settings-nav-sync'));
        await tester.ensureVisible(key('connect-error'));
        await settle(tester);
        await shot('$tag-cert-settings-error');
        await tester.ensureVisible(key('trusted-certificates'));
        await settle(tester);
        await shot('$tag-cert-settings-list');
        await tapAndSettle(tester, key('cert-pin-open-https://10.0.0.2:47100'));
        await shot('$tag-cert-view-trusted');
        expect(hubs.calls, isNotEmpty);
        await tv.app.sync.disconnect();
      });
    }
  }
}
