import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/sync/pairing_codec.dart';

void main() {
  tearDown(() => debugHubAddresses = null);

  test('no reachable interfaces offers no pairing URL', () async {
    debugHubAddresses = [];
    expect(await hubUrls('0.0.0.0:47100'), isEmpty);
    expect(await hubUrls('[::]:47100'), isEmpty);
  });

  test('loopback listeners cannot pair another device', () async {
    for (final host in ['127.0.0.1', '127.1.2.3', 'localhost', '[::1]']) {
      expect(await hubUrls('$host:47100'), isEmpty, reason: host);
    }
  });

  test('pairing URLs keep reachable interfaces and the bound port', () async {
    debugHubAddresses = const [
      HubAddress('127.0.0.1', AddressKind.loopback, 'lo'),
      HubAddress('100.100.100.100', AddressKind.tailscale, 'mesh'),
      HubAddress('192.168.0.5', AddressKind.lan, 'ethernet'),
    ];
    expect(await hubUrls('0.0.0.0:12345'), [
      ('http://100.100.100.100:12345', 'Tailscale'),
      ('http://192.168.0.5:12345', 'Local network'),
    ]);
    expect(await hubUrls('192.168.0.5:12345'), [
      ('http://192.168.0.5:12345', 'Hub address'),
    ]);
  });

  const token = '0123456789abcdef0123456789abcdef';
  for (final url in [
    'https://notes.example.com',
    'https://host:8443',
    'http://192.168.0.5:47100',
  ]) {
    test('pairing accepts $url without changing its port', () {
      for (final text in [Pairing(url, token).encode(), '$url $token']) {
        final paired = Pairing.parse(text)!;
        expect(paired.url, url);
        expect(paired.token, token);
      }
      expect(normalizeHubUrl('$url/'), url);
    });
  }

  test('old bare LAN address keeps its explicit port', () {
    expect(
      Pairing.parse('192.168.0.5:47100 $token')?.url,
      'http://192.168.0.5:47100',
    );
    expect(
      Pairing.parse('notes://pair?hub=http://192.168.0.5:47100&token=$token')
          ?.url,
      'http://192.168.0.5:47100',
    );
  });
}
