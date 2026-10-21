import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/sync/pairing_codec.dart';

void main() {
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
