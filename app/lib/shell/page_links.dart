import 'dart:async';
import 'dart:io';

import 'package:app_links/app_links.dart';

import '../core/page_link.dart';

/// Page links that reach the app from outside: the link that started it (cold
/// start) and later ones, forwarded by the running Windows instance or sent by
/// Android. Every other link, such as the `notes://auth` sign-in callback, is
/// ignored here and left to its own listener.
class PageLinkListener {
  PageLinkListener(this._onLink, {AppLinks? links})
    : _links =
          links ??
          (Platform.environment.containsKey('FLUTTER_TEST')
              ? null
              : AppLinks());

  final void Function(PageLink link) _onLink;

  /// Null under `flutter test`, which has no app_links plugin.
  final AppLinks? _links;

  /// Whether links can arrive at all; false under `flutter test`.
  bool get enabled => _links != null;
  StreamSubscription<Uri>? _sub;

  /// Subscribes to later links, then returns the page link that started the
  /// app, or null. Errors from the platform count as no link.
  Future<PageLink?> start() async {
    final links = _links;
    if (links == null) return null;
    _sub ??= links.uriLinkStream.listen(_receive, onError: (_) {});
    try {
      final initial = await links.getInitialLink();
      return initial == null ? null : PageLink.fromUri(initial);
    } catch (_) {
      return null;
    }
  }

  void _receive(Uri uri) {
    final link = PageLink.fromUri(uri);
    if (link != null) _onLink(link);
  }

  void dispose() {
    _sub?.cancel();
    _sub = null;
  }
}
