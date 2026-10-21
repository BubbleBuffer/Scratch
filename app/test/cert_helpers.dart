// A fake sync client for the certificate tests: the hub presents [chain]
// (leaf first); a sync succeeds when the leaf's fingerprint is trusted and
// otherwise fails as the bridge does ("untrusted certificate: <sha256>").
// SyncStore.clientFactory builds it; [FakeHubs.calls] records each client.

import 'package:flutter/widgets.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/features/sync/cert_trust.dart';
import 'package:notes_app/shell/home.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/state/sync_store.dart';

const leafSha = 'AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89';
const caSha = '11:22:33:44:55:66:77:88:99:00:AA:BB:CC:DD:EE:FF:11:22:33:44:55:66:77:88:99:00:AA:BB:CC:DD:EE:FF';
const otherSha = '0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F:0F';
const hubUrl = 'https://hub.home-lab.example.test:8443';

/// 2026-03-01 to 2027-03-01, noon UTC (the same day in any time zone).
const validFrom = 1772366400000, validUntil = 1803902400000;

const leafCert = CertificateInfo(
  subject: 'CN=hub.home-lab.example.test,O=Example Household Infrastructure,OU=Home Lab,L=Utrecht,C=NL',
  issuer: 'CN=Home Lab Private Root CA 2026,O=Example Household Infrastructure,C=NL',
  notBeforeMs: validFrom,
  notAfterMs: validUntil,
  sha256: leafSha,
  serial: '4F2A9C0B7D3E18A65C0E92F1B4D7A3C6',
  selfSigned: false,
);

const caCert = CertificateInfo(
  subject: 'CN=Home Lab Private Root CA 2026,O=Example Household Infrastructure,C=NL',
  issuer: 'CN=Home Lab Private Root CA 2026,O=Example Household Infrastructure,C=NL',
  notBeforeMs: 1735689600000,
  notAfterMs: 2051222400000,
  sha256: caSha,
  serial: '01',
  selfSigned: true,
);

class FakeHubs {
  FakeHubs({this.chain = const [leafCert, caCert], this.error});

  /// What the hub presents (leaf first).
  List<CertificateInfo> chain;

  /// Another failure instead of the certificate's (e.g. "hub connection").
  String? error;

  /// (url, trusted) per client built.
  final calls = <(String, List<String>)>[];

  void install() => SyncStore.clientFactory = (vault, url, token, trusted) {
        calls.add((url, List.of(trusted)));
        return FakeSync(this, trusted);
      };

  static void uninstall() => SyncStore.clientFactory =
      (vault, url, token, trusted) => NotesSync.withTrusted(vault: vault, url: url, token: token, trusted: trusted);
}

class FakeSync implements NotesSync {
  FakeSync(this.hubs, this.trusted);
  final FakeHubs hubs;
  final List<String> trusted;
  bool _disposed = false;

  bool get _ok =>
      hubs.error == null &&
      trusted.any((t) => normalizeFingerprint(t) == normalizeFingerprint(hubs.chain.first.sha256));

  @override
  SyncStatus status() => _ok
      ? const SyncStatus(running: true, syncing: false, syncs: 1, lastOkMs: validFrom)
      : SyncStatus(
          running: true,
          syncing: false,
          syncs: 0,
          lastError: hubs.error ?? 'untrusted certificate: ${hubs.chain.first.sha256}',
        );

  @override
  List<CertificateInfo>? untrustedCertificate() => _ok || hubs.error != null ? null : hubs.chain;

  @override
  Future<void> start({required SyncOptions options}) async {}

  @override
  Future<void> stop() async {}

  @override
  void syncNow() {}

  @override
  void dispose() => _disposed = true;

  @override
  bool get isDisposed => _disposed;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// A context under the app's providers.
BuildContext homeContext(WidgetTester tester) => tester.element(find.byType(HomeScreen));
