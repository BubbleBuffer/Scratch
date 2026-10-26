// Trusting a hub certificate with an unknown issuer, with a fake client
// (cert_helpers.dart) on the real app:
// - an untrusted-certificate error is an error, not offline;
// - the error shows "View certificate" and "Trust certificate" in the
//   indicator details and in Sync settings; Trust → confirm saves the pin
//   and reconnects with its fingerprint;
// - a changed certificate brings the prompt back with its own copy;
// - the certificate view shows every field at phone and desktop width;
// - revoking from the trusted list removes the pin and reconnects.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart' show Text;
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/cert_trust.dart';
import 'package:notes_app/features/sync/certificate_view.dart';
import 'package:notes_app/features/sync/sync_control.dart';
import 'package:notes_app/state/sync_store.dart';

import 'cert_helpers.dart';
import 'harness.dart';
import 'sync_helpers.dart';

void main() {
  setUpAll(initCore);
  tearDown(() {
    FakeHubs.uninstall();
    final s = AppSettings.memory;
    s.clientUrl = null;
    s.clientToken = null;
    s.trustedCertificates = TrustedCertificates();
  });

  Future<(SyncStore, AppSettings, FakeHubs)> connectFake(WidgetTester tester, {FakeHubs? hubs}) async {
    final h = (hubs ?? FakeHubs())..install();
    final sync = current.app.sync;
    final s = AppSettings.read(homeContext(tester));
    await connectToHub(sync, s, hubUrl, 'ab' * 32);
    await settle(tester);
    return (sync, s, h);
  }

  appTest('an untrusted certificate is an error; a hub connection failure is offline', (tester) async {
    await pumpApp(tester, mobile: false);
    final (sync, _, hubs) = await connectFake(tester);
    expect(sync.phase.$1, SyncPhase.error);
    expect(sync.untrustedCertificate, isTrue);
    expect(sync.untrustedFingerprint, leafSha);
    expect(sync.untrustedChain!.map((c) => c.sha256), [leafSha, caSha]);
    expect(hubs.calls.single.$1, hubUrl);
    expect(hubs.calls.single.$2, isEmpty);

    final (sync2, _, _) = await connectFake(tester, hubs: FakeHubs(error: 'hub connection: connection refused'));
    expect(sync2.phase.$1, SyncPhase.offline);
    expect(sync2.untrustedCertificate, isFalse);
    expect(sync2.untrustedChain, isNull);
    await sync2.disconnect();
  });

  appTest('trust from the indicator: confirm, pin saved, reconnect with the fingerprint', (tester) async {
    await pumpApp(tester, mobile: false);
    final (sync, s, hubs) = await connectFake(tester);

    await tapAndSettle(tester, key('sync-indicator'));
    expect(textOf(tester, 'sync-error-message'), "Can't verify the certificate for hub.home-lab.example.test:8443.");
    expect(find.textContaining('Sync is paused; your notes are safe on this device.'), findsOneWidget);
    expect(key('cert-view'), findsOneWidget);
    expect(key('cert-trust'), findsOneWidget);

    await tapAndSettle(tester, key('cert-trust'));
    expect(key('cert-trust-confirm'), findsOneWidget);
    expect(find.text('Trust this certificate?'), findsOneWidget);
    expect(find.textContaining('hub.home-lab.example.test:8443'), findsWidgets);
    expect(textOf(tester, 'cert-confirm-fingerprint').replaceAll('\n', ''), leafSha);

    // Cancel keeps everything as it was.
    await tapAndSettle(tester, key('confirm-cancel'));
    expect(s.trustedCertificates.isEmpty, isTrue);
    expect(hubs.calls, hasLength(1));

    await tapAndSettle(tester, key('sync-indicator'));
    await tapAndSettle(tester, key('cert-trust'));
    await tapAndSettle(tester, key('confirm-ok'));
    final pin = s.trustedCertificates.lookup(hubUrl)!;
    expect(pin.sha256, leafSha);
    expect(pin.subject, leafCert.subject);
    expect(pin.chain.map((c) => c.sha256), [leafSha, caSha]);
    expect(hubs.calls.last.$1, hubUrl);
    expect(hubs.calls.last.$2, [leafSha]);
    expect([for (final t in sync.trusted) t.sha256], [leafSha]);
    expect(sync.untrustedCertificate, isFalse);
    expect(sync.phase.$1, SyncPhase.synced);
    await sync.disconnect();
  });

  appTest('Sync settings shows the error; trust from the certificate view', (tester) async {
    await pumpApp(tester, mobile: false);
    final (sync, s, hubs) = await connectFake(tester);
    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('settings-nav-sync'));
    expect(textOf(tester, 'connect-error'), "Can't verify the certificate for hub.home-lab.example.test:8443.");
    expect(key('cert-view'), findsOneWidget);
    expect(key('cert-trust'), findsOneWidget);
    expect(key('trusted-certificates'), findsNothing);

    await tapAndSettle(tester, key('cert-view'));
    expect(textOf(tester, 'cert-status'), 'Not trusted — unknown issuer');
    await tapAndSettle(tester, key('cert-view-trust'));
    await tapAndSettle(tester, key('confirm-ok'));
    expect(key('cert-view-body'), findsNothing, reason: 'the view closes once trusted');
    expect(s.trustedCertificates.lookup(hubUrl)?.sha256, leafSha);
    expect(hubs.calls.last.$2, [leafSha]);
    expect(key('connect-error'), findsNothing);
    expect(key('cert-pin-https://hub.home-lab.example.test:8443'), findsOneWidget);
    // The issuer the hub sent is kept and passed on as this origin's anchor.
    final pin = s.trustedCertificates.lookup(hubUrl)!;
    expect(pin.issuerDer, caDer);
    expect(pin.issuerSha256, caSha);
    final passed = hubs.trust.last.single;
    expect(passed.origin, 'https://hub.home-lab.example.test:8443');
    expect(passed.issuerDer, caDer);
    await sync.disconnect();
  });

  appTest('trusted certificates name the issuing CA; an entry saved before issuers were kept still shows',
      (tester) async {
    await pumpApp(tester, mobile: false);
    final s = AppSettings.read(homeContext(tester));
    const origin = 'https://notes.corp.example.test:443';
    // The settings shape from before issuers were kept: no issuerDer.
    final old = TrustedCertificates.fromJson({
      origin: {
        'sha256': '05:3B:4F:48:4C:22:${leafSha.substring(18)}',
        'subject': 'CN=notes.corp.example.test',
        'issuer': 'CN=Example Proxy CA,O=Example Corp,OU=Network Security',
        'notBeforeMs': validFrom,
        'notAfterMs': validUntil,
        'trustedAtMs': validFrom,
        'chain': [],
      },
      'https://hub.example.test:443': {
        'sha256': leafSha,
        'subject': 'CN=hub.example.test',
        'issuer': 'O=No Name Inc',
        'notAfterMs': validUntil,
        'trustedAtMs': validFrom,
      },
    });
    expect(old.lookup(origin)!.issuerDer, isNull);
    s.update((s) => s.trustedCertificates = old);
    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('settings-nav-sync'));
    await tester.ensureVisible(key('cert-pin-$origin'));
    await settle(tester);

    expect(textOf(tester, 'cert-pin-title-$origin'), 'Example Proxy CA');
    expect(textOf(tester, 'cert-pin-org-$origin'), 'Example Corp · Network Security');
    final detail = tester.widget<Text>(key('cert-pin-detail-$origin')).textSpan!.toPlainText();
    expect(detail, 'for notes.corp.example.test · 05:3B:4F:48:4C:22… · valid until Mar 1, 2027');

    // No CN: the host stays the title, as before.
    const other = 'https://hub.example.test:443';
    expect(textOf(tester, 'cert-pin-title-$other'), 'hub.example.test');
    expect(key('cert-pin-org-$other'), findsNothing);
    final otherDetail = tester.widget<Text>(key('cert-pin-detail-$other')).textSpan!.toPlainText();
    expect(otherDetail, 'AB:CD:EF:01:23:45… · valid until Mar 1, 2027');
  });

  appTest('a changed certificate brings the prompt back', (tester) async {
    await pumpApp(tester, mobile: false);
    final s = AppSettings.read(homeContext(tester));
    s.update((s) => s.trustedCertificates.add(
        hubUrl, PinnedCertificate.of(const [CertDetails(sha256: otherSha)], trustedAtMs: 1)));
    final (sync, _, hubs) = await connectFake(tester);
    expect(hubs.calls.single.$2, [otherSha], reason: 'the pin is passed when connecting');
    expect(sync.untrustedCertificate, isTrue);

    await tapAndSettle(tester, key('sync-indicator'));
    expect(textOf(tester, 'sync-error-message'),
        'The certificate for hub.home-lab.example.test:8443 changed since you trusted it.');
    expect(find.text('Trust the new one only if you changed it yourself.'), findsOneWidget);
    await tapAndSettle(tester, key('cert-view'));
    expect(textOf(tester, 'cert-status'), 'Changed since you trusted it');
    await tapAndSettle(tester, key('cert-view-trust'));
    await tapAndSettle(tester, key('confirm-ok'));
    expect(s.trustedCertificates.lookup(hubUrl)!.sha256, leafSha, reason: 'the new one replaces the pin');
    expect(hubs.calls.last.$2, [leafSha]);
    await sync.disconnect();
  });

  appTest('revoke from the trusted list removes the pin and reconnects', (tester) async {
    await pumpApp(tester, mobile: false);
    final s = AppSettings.read(homeContext(tester));
    s.update((s) => s.trustedCertificates.add(
        hubUrl, PinnedCertificate.of([certDetailsOf(leafCert), certDetailsOf(caCert)], trustedAtMs: validFrom)));
    final (sync, _, hubs) = await connectFake(tester);
    expect(sync.phase.$1, SyncPhase.synced);

    await tapAndSettle(tester, key('sidebar-settings'));
    await tapAndSettle(tester, key('settings-nav-sync'));
    const origin = 'https://hub.home-lab.example.test:8443';
    expect(find.textContaining('AB:CD:EF:01:23:45…'), findsOneWidget);
    expect(find.textContaining('valid until Mar 1, 2027'), findsOneWidget);

    // The row opens the stored certificate.
    await tester.ensureVisible(key('cert-pin-open-$origin'));
    await settle(tester);
    await tapAndSettle(tester, key('cert-pin-open-$origin'));
    expect(textOf(tester, 'cert-status'), 'Trusted since Mar 1, 2026');
    expect(key('cert-revoke'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle(tester);

    await tester.ensureVisible(key('cert-pin-revoke-$origin'));
    await settle(tester);
    await tapAndSettle(tester, key('cert-pin-revoke-$origin'));
    expect(find.text('Stop trusting the certificate for hub.home-lab.example.test:8443?'), findsOneWidget);
    expect(find.text('Sync with this hub stops until you trust it again.'), findsOneWidget);
    await tapAndSettle(tester, key('confirm-ok'));
    expect(s.trustedCertificates.isEmpty, isTrue);
    expect(key('trusted-certificates'), findsNothing);
    expect(hubs.calls.last.$2, isEmpty);
    expect(sync.untrustedCertificate, isTrue);
    await sync.disconnect();
  });

  for (final mobile in [true, false]) {
    appTest('the certificate view shows every field (${mobile ? '375 px' : 'desktop'})', (tester) async {
      await pumpApp(tester, mobile: mobile);
      final expiredLeaf = CertDetails(
        subject: leafCert.subject,
        issuer: leafCert.issuer,
        notBeforeMs: 1609502400000,
        notAfterMs: 1641038400000,
        sha256: leafSha,
        serial: leafCert.serial,
      );
      showCertificate(homeContext(tester),
          host: 'hub.home-lab.example.test:8443',
          status: CertStatus.untrusted,
          chain: [expiredLeaf, certDetailsOf(caCert)],
          onTrust: (_) async => false);
      await settle(tester);
      expect(textOf(tester, 'cert-host'), 'hub.home-lab.example.test:8443');
      expect(textOf(tester, 'cert-status'), 'Not trusted — unknown issuer');
      for (final f in ['subject', 'issuer', 'from', 'until', 'sha256', 'serial']) {
        expect(key('cert-leaf-$f'), findsOneWidget, reason: f);
      }
      expect(key('cert-leaf-self-signed'), findsNothing);
      expect(find.text(leafCert.subject), findsWidgets);
      expect(find.text('Jan 1, 2022 · Expired'), findsOneWidget);
      expect(textOf(tester, 'cert-leaf-sha256-text').replaceAll('\n', ''), leafSha);
      expect(key('cert-copy'), findsOneWidget);
      expect(find.text('Chain'), findsOneWidget);
      expect(key('cert-chain-row-0'), findsOneWidget);
      expect(key('cert-chain-row-1'), findsOneWidget);
      expect(key('cert-view-trust'), findsOneWidget);
      expect(key('cert-revoke'), findsNothing);

      await tester.ensureVisible(key('cert-chain-row-1'));
      await tapAndSettle(tester, key('cert-chain-row-1'));
      await tester.ensureVisible(key('cert-chain-1-self-signed'));
      expect(key('cert-chain-1-self-signed'), findsOneWidget);
      expect(textOf(tester, 'cert-chain-1-sha256-text').replaceAll('\n', ''), caSha);
      expect(tester.takeException(), isNull, reason: 'no overflow');
    });
  }
}
