// Hub addresses and pairing strings.
//
// - A pairing string carries the hub URL and a token in one line (also the
//   QR code's content): `notes://pair?hub=http://100.101.102.103:47100&token=...`.
// - [hubAddresses] lists this machine's IPv4 addresses that other devices
//   can use: Tailscale (100.64.0.0/10) first, then private LAN ranges.

import 'dart:io';

/// A hub URL paired with a token.
class Pairing {
  const Pairing(this.url, this.token);
  final String url;
  final String token;

  /// The one-line form (QR code content).
  String encode() => 'notes://pair?hub=$url&token=$token';

  /// Reads a pairing string, or "url token" separated by whitespace.
  /// Returns null if [text] is neither.
  static Pairing? parse(String text) {
    final t = text.trim();
    if (t.startsWith('notes://pair')) {
      final q = t.indexOf('?');
      if (q < 0) return null;
      String? hub, token;
      for (final part in t.substring(q + 1).split('&')) {
        final eq = part.indexOf('=');
        if (eq < 0) continue;
        final k = part.substring(0, eq);
        final v = Uri.decodeQueryComponent(part.substring(eq + 1));
        if (k == 'hub') hub = v;
        if (k == 'token') token = v;
      }
      if (hub == null || token == null || hub.isEmpty || token.isEmpty) return null;
      return Pairing(normalizeHubUrl(hub), token);
    }
    final parts = t.split(RegExp(r'\s+'));
    if (parts.length == 2 && _looksLikeHost(parts[0]) && RegExp(r'^[0-9a-fA-F]{16,}$').hasMatch(parts[1])) {
      return Pairing(normalizeHubUrl(parts[0]), parts[1]);
    }
    return null;
  }

  static bool _looksLikeHost(String s) => s.startsWith('http://') || s.startsWith('https://') || RegExp(r'^[\w.\-]+:\d+$').hasMatch(s);
}

/// Adds `http://` when missing and drops a trailing slash.
String normalizeHubUrl(String s) {
  var u = s.trim();
  if (u.isEmpty) return u;
  if (!u.contains('://')) u = 'http://$u';
  while (u.endsWith('/')) {
    u = u.substring(0, u.length - 1);
  }
  return u;
}

enum AddressKind { tailscale, lan, loopback }

class HubAddress {
  const HubAddress(this.ip, this.kind, this.interface);
  final String ip;
  final AddressKind kind;
  final String interface;

  String get kindLabel => switch (kind) {
        AddressKind.tailscale => 'Tailscale',
        AddressKind.lan => 'Local network',
        AddressKind.loopback => 'This computer',
      };

  String url(int port) => 'http://$ip:$port';
}

/// Test hook: replaces the interface scan (screenshots need stable data).
List<HubAddress>? debugHubAddresses;

bool _tailscale(List<int> b) => b[0] == 100 && (b[1] & 0xC0) == 64;
bool _private(List<int> b) =>
    b[0] == 10 || (b[0] == 172 && b[1] >= 16 && b[1] <= 31) || (b[0] == 192 && b[1] == 168);

/// This machine's reachable IPv4 addresses, Tailscale first.
Future<List<HubAddress>> hubAddresses() async {
  final forced = debugHubAddresses;
  if (forced != null) return forced;
  final out = <HubAddress>[];
  try {
    final ifs = await NetworkInterface.list(type: InternetAddressType.IPv4);
    for (final i in ifs) {
      for (final a in i.addresses) {
        final b = a.rawAddress;
        if (b.length != 4) continue;
        if (_tailscale(b)) {
          out.add(HubAddress(a.address, AddressKind.tailscale, i.name));
        } else if (_private(b)) {
          out.add(HubAddress(a.address, AddressKind.lan, i.name));
        }
      }
    }
  } catch (_) {}
  out.sort((a, b) => a.kind.index.compareTo(b.kind.index));
  return out;
}

/// The port of a bound or configured address ("0.0.0.0:47100" -> 47100).
int portOf(String addr) => int.tryParse(addr.substring(addr.lastIndexOf(':') + 1)) ?? 47100;

/// The host part of an address ("0.0.0.0:47100" -> "0.0.0.0").
String hostOf(String addr) => addr.substring(0, addr.lastIndexOf(':').clamp(0, addr.length));

/// URLs other devices can use for a hub bound to [bound]: one per
/// reachable address when it listens on every interface, else its own.
Future<List<(String url, String label)>> hubUrls(String bound) async {
  final host = hostOf(bound);
  final port = portOf(bound);
  if (host == '0.0.0.0' || host == '[::]' || host == '::') {
    final list = await hubAddresses();
    if (list.isEmpty) return [('http://127.0.0.1:$port', 'This computer')];
    return [for (final a in list) (a.url(port), a.kindLabel)];
  }
  final ip = host == '127.0.0.1' || host == 'localhost' ? 'This computer' : 'Hub address';
  return [('http://$host:$port', ip)];
}
