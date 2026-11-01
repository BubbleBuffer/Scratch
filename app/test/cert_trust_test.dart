// The trust store for hub certificates with an unknown issuer (plain Dart):
// origin normalization, add/replace, revoke, lookup, a changed fingerprint,
// the error format, and the settings file round-trip (including an older
// file without the field).

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/settings/settings_store.dart';
import 'package:notes_app/features/sync/cert_trust.dart';

const fp1 = 'AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89';
const fp2 = '11:22:33:44:55:66:77:88:99:00:AA:BB:CC:DD:EE:FF:11:22:33:44:55:66:77:88:99:00:AA:BB:CC:DD:EE:FF';

PinnedCertificate pin(String fp, {int at = 1000}) => PinnedCertificate.of(
      [
        CertDetails(subject: 'CN=hub.example.test', issuer: 'CN=Home CA', notBeforeMs: 1, notAfterMs: 2, sha256: fp, serial: '0A1B'),
        const CertDetails(subject: 'CN=Home CA', issuer: 'CN=Home CA', sha256: fp2, selfSigned: true),
      ],
      trustedAtMs: at,
    );

void main() {
  group('issuer names and anchors', () {
    test('nameParts takes CN, O and OU in any order, unescaped', () {
      final n = nameParts('OU=Network Security,O=Example Corp,CN=Example Proxy CA');
      expect(n.cn, 'Example Proxy CA');
      expect(n.o, 'Example Corp');
      expect(n.ou, 'Network Security');
      final e = nameParts(r'CN=Proxy\, Inc+OU=Ops,O=A\2Bb,C=DE');
      expect(e.cn, 'Proxy, Inc');
      expect(e.ou, 'Ops');
      expect(e.o, 'A+b');
      final none = nameParts('O=No Name Inc');
      expect(none.cn, isNull);
      expect(nameParts('').cn, isNull);
    });

    test('a pin keeps the issuer DER the hub sent, and passes it on for its origin', () {
      final der = Uint8List.fromList([0x30, 0x82, 0x01, 0x0A]);
      final p = PinnedCertificate.of(
        [
          const CertDetails(subject: 'CN=notes.example.test', issuer: 'CN=Example Proxy CA,O=Example Corp', sha256: fp1),
          const CertDetails(subject: 'CN=Example Proxy CA,O=Example Corp', issuer: 'CN=Root', sha256: fp2),
        ],
        trustedAtMs: 1,
        issuerDer: der,
      );
      expect(p.issuerDer, der);
      expect(p.issuerSha256, fp2);
      expect(p.issuerName.cn, 'Example Proxy CA');

      final t = TrustedCertificates()..add('https://notes.example.test', p);
      final trust = t.trustFor('https://NOTES.example.test/x');
      expect(trust.single.origin, 'https://notes.example.test:443');
      expect(trust.single.sha256, fp1);
      expect(trust.single.issuerDer, der);
      expect(t.trustFor('https://other.example.test'), isEmpty);
      expect(t.trustFor('https://notes.example.test:8443'), isEmpty);

      // Round trip through the settings JSON.
      final back = TrustedCertificates.fromJson(jsonDecode(jsonEncode(t.toJson())));
      final q = back.lookup('https://notes.example.test')!;
      expect(q.issuerDer, der);
      expect(q.issuerSha256, fp2);
      expect(back.trustKeyFor('https://notes.example.test'), t.trustKeyFor('https://notes.example.test'));
    });

    test('no issuer DER without an issuer in the chain', () {
      final p = PinnedCertificate.of([const CertDetails(sha256: fp1)],
          trustedAtMs: 1, issuerDer: Uint8List.fromList([1, 2, 3]));
      expect(p.issuerDer, isNull);
      expect(p.toJson().containsKey('issuerDer'), isFalse);
    });

    test('an entry saved before issuers were kept loads, with only its leaf pin', () {
      final t = TrustedCertificates.fromJson({
        'https://notes.corp.example.test:443': {
          'sha256': fp1,
          'subject': 'CN=notes.corp.example.test',
          'issuer': 'CN=Example Proxy CA,O=Example Corp,OU=Network Security',
          'notBeforeMs': 1,
          'notAfterMs': 2,
          'trustedAtMs': 3,
          'chain': [],
        },
      });
      final p = t.lookup('https://notes.corp.example.test')!;
      expect(p.issuerDer, isNull);
      expect(p.issuerSha256, '');
      expect(p.issuerName.cn, 'Example Proxy CA');
      expect(p.issuerName.o, 'Example Corp');
      expect(p.issuerName.ou, 'Network Security');
      final trust = t.trustFor('https://notes.corp.example.test').single;
      expect(trust.sha256, fp1);
      expect(trust.issuerDer, isNull);
      expect(t.toJson()['https://notes.corp.example.test:443'], isNot(contains('issuerDer')));
    });
  });

  group('originOf', () {
    test('lowercases scheme and host and fills in the default port', () {
      expect(originOf('https://Hub.Example.TEST/sync'), 'https://hub.example.test:443');
      expect(originOf('HTTP://hub.example.test'), 'http://hub.example.test:80');
      expect(originOf('https://hub.example.test:443'), 'https://hub.example.test:443');
      expect(originOf('https://hub.example.test:8443/'), 'https://hub.example.test:8443');
    });
    test('a bare host:port is http, as hub addresses are', () {
      expect(originOf('100.101.102.103:47100'), 'http://100.101.102.103:47100');
    });
    test('IPv6 keeps its brackets', () {
      expect(originOf('https://[::1]:8443'), 'https://[::1]:8443');
    });
    test('no host gives null', () {
      expect(originOf(''), isNull);
      expect(originOf('https://'), isNull);
    });
    test('hostOf drops the default port only', () {
      expect(serverHostOf('https://Hub.example.test'), 'hub.example.test');
      expect(serverHostOf('https://hub.example.test:8443'), 'hub.example.test:8443');
      expect(serverHostOf('http://hub.example.test:443'), 'hub.example.test:443');
    });
  });

  group('fingerprints', () {
    test('any case, colons optional', () {
      expect(normalizeFingerprint('ab:cd:ef'), 'ABCDEF');
      expect(formatFingerprint('abcdef'), 'AB:CD:EF');
      expect(shortFingerprint(fp1), 'AB:CD:EF:01:23:45…');
    });
    test('the error names the fingerprint', () {
      expect(isUntrustedCertificateError('untrusted certificate: $fp1'), isTrue);
      expect(isUntrustedCertificateError('hub connection: refused'), isFalse);
      expect(isUntrustedCertificateError(null), isFalse);
      expect(untrustedFingerprintOf('untrusted certificate: $fp1'), fp1);
      expect(untrustedFingerprintOf('untrusted certificate: ${fp1.toLowerCase().replaceAll(':', '')}'), fp1);
      expect(untrustedFingerprintOf('hub connection: refused'), isNull);
    });
  });

  group('TrustedCertificates', () {
    test('add, lookup by any URL of the origin, replace, revoke', () {
      final t = TrustedCertificates();
      expect(t.isEmpty, isTrue);
      expect(t.add('https://Hub.example.test/', pin(fp1)), 'https://hub.example.test:443');
      expect(t.lookup('https://hub.example.test:443/x')?.sha256, fp1);
      expect(t.fingerprintsFor('https://HUB.example.test'), [fp1]);
      expect(t.lookup('https://hub.example.test:8443'), isNull, reason: 'another port is another origin');
      expect(t.lookup('http://hub.example.test'), isNull, reason: 'another scheme is another origin');
      expect(t.fingerprintsFor('https://other.test'), isEmpty);

      t.add('https://hub.example.test', pin(fp2, at: 2000));
      expect(t.entries, hasLength(1));
      expect(t.lookup('https://hub.example.test')!.sha256, fp2);
      expect(t.lookup('https://hub.example.test')!.trustedAtMs, 2000);

      expect(t.revoke('https://HUB.example.test:443'), isTrue);
      expect(t.revoke('https://hub.example.test'), isFalse);
      expect(t.isEmpty, isTrue);
    });

    test('changed: a pin exists and the fingerprint differs', () {
      final t = TrustedCertificates()..add('https://hub.example.test', pin(fp1));
      expect(t.changed('https://hub.example.test', fp1), isFalse);
      expect(t.changed('https://hub.example.test', fp1.toLowerCase().replaceAll(':', '')), isFalse);
      expect(t.changed('https://hub.example.test', fp2), isTrue);
      expect(t.changed('https://other.test', fp2), isFalse, reason: 'no pin: untrusted, not changed');
      expect(t.changed('https://hub.example.test', null), isFalse);
    });

    test('pin keeps the leaf fields and the chain', () {
      final p = pin(fp1);
      expect(p.sha256, fp1);
      expect(p.subject, 'CN=hub.example.test');
      expect(p.issuer, 'CN=Home CA');
      expect(p.notAfterMs, 2);
      expect(p.chain, hasLength(2));
      expect(PinnedCertificate.of(const [CertDetails(sha256: 'aabb')], trustedAtMs: 1).sha256, 'AA:BB');
    });

    test('JSON round-trip; malformed entries are skipped', () {
      final t = TrustedCertificates()
        ..add('https://hub.example.test', pin(fp1, at: 42))
        ..add('http://10.0.0.2:47100', pin(fp2));
      final back = TrustedCertificates.fromJson(jsonDecode(jsonEncode(t.toJson())));
      expect(back.entries.map((e) => e.$1), ['http://10.0.0.2:47100', 'https://hub.example.test:443']);
      final p = back.lookup('https://hub.example.test')!;
      expect(p.sha256, fp1);
      expect(p.trustedAtMs, 42);
      expect(p.chain, pin(fp1).chain);
      expect(p.chain[1].selfSigned, isTrue);

      final odd = TrustedCertificates.fromJson({
        'https://a.test': {'sha256': fp1},
        'https://b.test': {'subject': 'no fingerprint'},
        'https://c.test': 'not a map',
        '': {'sha256': fp1},
      });
      expect(odd.entries.map((e) => e.$1), ['https://a.test:443']);
      expect(odd.lookup('https://a.test')!.viewChain.single.sha256, fp1);
      expect(TrustedCertificates.fromJson(null).isEmpty, isTrue);
      expect(TrustedCertificates.fromJson('junk').isEmpty, isTrue);
    });
  });

  group('AppSettings', () {
    late Directory dir;
    setUp(() => dir = Directory.systemTemp.createTempSync('notes_cert_'));
    tearDown(() => dir.deleteSync(recursive: true));

    test('trusted certificates survive a restart', () {
      final f = File('${dir.path}/settings.json');
      final s = AppSettings(file: f);
      s.update((s) => s.trustedCertificates.add('https://hub.example.test:8443', pin(fp1)));
      final again = AppSettings.load(f);
      expect(again.trustedCertificates.lookup('https://HUB.example.test:8443')?.sha256, fp1);
      expect(again.trustedCertificates.lookup('https://hub.example.test:8443')!.chain, hasLength(2));

      again.update((s) => s.trustedCertificates.revoke('https://hub.example.test:8443'));
      expect(AppSettings.load(f).trustedCertificates.isEmpty, isTrue);
      expect(jsonDecode(f.readAsStringSync()), isNot(contains('trustedCertificates')));
    });

    test('an older settings file without the field loads', () {
      final f = File('${dir.path}/settings.json')
        ..writeAsStringSync(jsonEncode({'theme': 'dark', 'clientUrl': 'https://hub.example.test', 'clientToken': 'ab'}));
      final s = AppSettings.load(f);
      expect(s.theme, ThemePref.dark);
      expect(s.clientUrl, 'https://hub.example.test');
      expect(s.trustedCertificates.isEmpty, isTrue);
    });
  });
}
