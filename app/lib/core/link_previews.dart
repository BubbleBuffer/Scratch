// What a web bookmark shows: the page's title, description and favicon.
// Fetched once per URL on a background isolate (a GET of at most 512 KB,
// then the favicon), kept in memory and, on this device, in
// `<data dir>/link_previews.json`. Offline or failing fetches show the plain
// card and are tried again on the next start; nothing is reported.
//
// Not stored in the vault: previews are a cache, so they do not sync.

import 'debounce.dart';

import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:flutter/foundation.dart';

/// A fetched preview.
class LinkPreview {
  const LinkPreview({this.title, this.description, this.icon});
  final String? title, description;

  /// The favicon's bytes (PNG, ICO, …), or null.
  final Uint8List? icon;

  bool get isEmpty => (title ?? '').isEmpty && (description ?? '').isEmpty && icon == null;

  Map<String, dynamic> toJson() => {
        'title': ?title,
        'description': ?description,
        if (icon != null) 'icon': base64Encode(icon!),
      };

  static LinkPreview fromJson(Map<String, dynamic> j) => LinkPreview(
        title: j['title'] as String?,
        description: j['description'] as String?,
        icon: j['icon'] == null ? null : base64Decode(j['icon'] as String),
      );
}

/// Fetches the preview of a URL (null when it cannot).
typedef PreviewFetch = Future<LinkPreview?> Function(String url);

class LinkPreviews extends ChangeNotifier {
  /// [file] keeps previews across starts (memory only if null). [fetch]
  /// defaults to [fetchLinkPreview] on a background isolate (none under
  /// `flutter test`).
  LinkPreviews({this.file, PreviewFetch? fetch}) : _fetch = fetch ?? _defaultFetch {
    _load();
  }

  final File? file;
  final PreviewFetch _fetch;

  static const maxEntries = 300;
  static const saveDelay = Duration(milliseconds: 500);

  final LinkedHashMap<String, LinkPreview> _cache = LinkedHashMap();
  final Set<String> _pending = {};
  final Set<String> _failed = {};
  Timer? _save;
  bool _disposed = false;

  static Future<LinkPreview?> _defaultFetch(String url) async {
    if (Platform.environment.containsKey('FLUTTER_TEST')) return null;
    return Isolate.run(() => fetchLinkPreview(url));
  }

  /// The preview of [url], or null while it loads (a notification follows)
  /// or when there is none.
  LinkPreview? of(String url) {
    final p = _cache[url];
    if (p != null) return p;
    if (_failed.contains(url) || !_pending.add(url)) return null;
    _fetch(url).then((p) {
      _pending.remove(url);
      if (_disposed) return;
      if (p == null || p.isEmpty) {
        _failed.add(url);
        return;
      }
      _cache[url] = p;
      while (_cache.length > maxEntries) {
        _cache.remove(_cache.keys.first);
      }
      _scheduleSave();
      notifyListeners();
    }, onError: (Object _) {
      _pending.remove(url);
      _failed.add(url);
    });
    return null;
  }

  void _load() {
    final f = file;
    if (f == null) return;
    try {
      if (!f.existsSync()) return;
      final j = jsonDecode(f.readAsStringSync()) as Map<String, dynamic>;
      for (final e in j.entries) {
        _cache[e.key] = LinkPreview.fromJson(e.value as Map<String, dynamic>);
      }
    } catch (e) {
      debugPrint('link previews unreadable: $e');
    }
  }

  void _scheduleSave() {
    if (file == null) return;
    _save?.cancel();
    _save = Timer(Debounce.testDelay(saveDelay), flush);
  }

  /// Writes the cache file now.
  void flush() {
    _save?.cancel();
    _save = null;
    final f = file;
    if (f == null) return;
    try {
      f.parent.createSync(recursive: true);
      final tmp = File('${f.path}.tmp');
      tmp.writeAsStringSync(jsonEncode({for (final e in _cache.entries) e.key: e.value.toJson()}), flush: true);
      tmp.renameSync(f.path);
    } catch (e) {
      debugPrint('link previews not saved: $e');
    }
  }

  @override
  void dispose() {
    if (_save != null) flush();
    _disposed = true;
    super.dispose();
  }
}

// ---------------------------------------------------------------------------
// Fetching (runs on a background isolate).

const _maxPage = 512 * 1024, _maxIcon = 100 * 1024;
const _timeout = Duration(seconds: 8);

/// Fetches [url] and reads its title, description and favicon.
Future<LinkPreview?> fetchLinkPreview(String url) async {
  final uri = Uri.tryParse(url);
  if (uri == null || !(uri.scheme == 'http' || uri.scheme == 'https') || uri.host.isEmpty) return null;
  final client = HttpClient()
    ..connectionTimeout = _timeout
    ..userAgent = 'Mozilla/5.0 (compatible; NotesLinkPreview/1.0)';
  try {
    final page = await _get(client, uri, _maxPage, html: true);
    if (page == null) return null;
    final (base, bytes) = page;
    final html = utf8.decode(bytes, allowMalformed: true);
    final meta = parseLinkPreview(html, base);
    Uint8List? icon;
    for (final u in meta.icons) {
      final r = await _get(client, u, _maxIcon);
      if (r != null && r.$2.isNotEmpty) {
        icon = r.$2;
        break;
      }
    }
    return LinkPreview(title: meta.title, description: meta.description, icon: icon);
  } catch (_) {
    return null;
  } finally {
    client.close(force: true);
  }
}

Future<(Uri, Uint8List)?> _get(HttpClient client, Uri uri, int max, {bool html = false}) async {
  final req = await client.getUrl(uri).timeout(_timeout);
  req.followRedirects = true;
  req.maxRedirects = 5;
  final res = await req.close().timeout(_timeout);
  if (res.statusCode != 200) {
    await res.drain<void>().catchError((_) {});
    return null;
  }
  final type = res.headers.contentType?.mimeType ?? '';
  if (html && type.isNotEmpty && !type.contains('html')) {
    await res.drain<void>().catchError((_) {});
    return null;
  }
  final out = BytesBuilder(copy: false);
  await for (final chunk in res.timeout(_timeout)) {
    out.add(chunk);
    if (out.length >= max) break;
  }
  final last = res.redirects.isEmpty ? uri : uri.resolveUri(res.redirects.last.location);
  return (last, out.takeBytes());
}

/// What [parseLinkPreview] finds in a page.
class ParsedPreview {
  const ParsedPreview(this.title, this.description, this.icons);
  final String? title, description;

  /// Favicon candidates, best first.
  final List<Uri> icons;
}

final _tagRe = RegExp(r'<(meta|link)\b[^>]*>', caseSensitive: false);
final _attrRe = RegExp(r'''([a-zA-Z:-]+)\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))''');
final _titleRe = RegExp(r'<title[^>]*>([\s\S]*?)</title>', caseSensitive: false);

/// Reads the title, description and favicons from [html] (served at [base]).
ParsedPreview parseLinkPreview(String html, Uri base) {
  final head = html.length > 200000 ? html.substring(0, 200000) : html;
  final metas = <String, String>{};
  final icons = <(int, Uri)>[];
  for (final t in _tagRe.allMatches(head)) {
    final attrs = <String, String>{};
    for (final a in _attrRe.allMatches(t.group(0)!)) {
      attrs[a.group(1)!.toLowerCase()] = _unescape(a.group(3) ?? a.group(4) ?? a.group(5) ?? '');
    }
    if (t.group(1)!.toLowerCase() == 'meta') {
      final k = (attrs['property'] ?? attrs['name'] ?? '').toLowerCase();
      final v = attrs['content'];
      if (k.isNotEmpty && v != null && v.trim().isNotEmpty) metas.putIfAbsent(k, () => v.trim());
    } else {
      final rel = (attrs['rel'] ?? '').toLowerCase().split(RegExp(r'\s+'));
      final href = attrs['href'];
      if (href == null || href.isEmpty || !rel.any((r) => r == 'icon' || r == 'apple-touch-icon')) continue;
      final u = base.resolve(href);
      if (u.scheme != 'http' && u.scheme != 'https') continue;
      final path = u.path.toLowerCase();
      // PNG first, then ICO; SVG cannot be shown.
      if (path.endsWith('.svg') || (attrs['type'] ?? '').contains('svg')) continue;
      final rank = path.endsWith('.png') ? 0 : (rel.contains('apple-touch-icon') ? 1 : 2);
      icons.add((rank, u));
    }
  }
  icons.sort((a, b) => a.$1.compareTo(b.$1));
  final title = metas['og:title'] ?? metas['twitter:title'] ?? _clean(_titleRe.firstMatch(head)?.group(1));
  final description = metas['og:description'] ?? metas['description'] ?? metas['twitter:description'];
  return ParsedPreview(
    _short(title, 200),
    _short(description, 400),
    [for (final i in icons) i.$2, base.resolve('/favicon.ico')],
  );
}

String? _clean(String? s) => s == null ? null : _unescape(s.replaceAll(RegExp(r'\s+'), ' ').trim());

String? _short(String? s, int n) {
  if (s == null || s.isEmpty) return null;
  final t = s.replaceAll(RegExp(r'\s+'), ' ').trim();
  return t.length > n ? '${t.substring(0, n)}…' : t;
}

String _unescape(String s) => s.replaceAllMapped(RegExp(r'&(#x[0-9a-fA-F]+|#[0-9]+|[a-zA-Z]+);'), (m) {
      final e = m.group(1)!;
      if (e.startsWith('#x') || e.startsWith('#X')) {
        final c = int.tryParse(e.substring(2), radix: 16);
        return c == null ? m.group(0)! : String.fromCharCode(c);
      }
      if (e.startsWith('#')) {
        final c = int.tryParse(e.substring(1));
        return c == null ? m.group(0)! : String.fromCharCode(c);
      }
      return switch (e.toLowerCase()) {
        'amp' => '&',
        'lt' => '<',
        'gt' => '>',
        'quot' => '"',
        'apos' => "'",
        'nbsp' => ' ',
        _ => m.group(0)!,
      };
    });
