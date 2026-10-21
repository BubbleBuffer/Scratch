/// `notes://page/<page id>` links, optionally `?block=<block id>` to land on a
/// block. This is the one parser for every entry point: the Windows and
/// Android link handlers, editor page marks and the agent chat. The format is
/// what the page menu's Copy link writes, so pasted links and launch
/// arguments read the same way. Pure Dart, so it has no bridge dependency.
class PageLink {
  const PageLink(this.pageId, {this.blockId});

  static const scheme = 'notes';
  static const host = 'page';

  /// Shown when a link names a page that does not exist or is in the trash.
  static const missingMessage = 'No page matches that link.';

  final String pageId;
  final String? blockId;

  /// The link text: `notes://page/<id>`, plus `?block=<id>` when [blockId] is
  /// set.
  String get url => Uri(
    scheme: scheme,
    host: host,
    pathSegments: [pageId],
    queryParameters: blockId == null ? null : {'block': blockId!},
  ).toString();

  /// The page link in [text], or null when [text] is not one. Surrounding
  /// spaces are ignored.
  static PageLink? parse(String text) {
    final uri = Uri.tryParse(text.trim());
    return uri == null ? null : fromUri(uri);
  }

  /// The page link [uri] is, or null. Two forms are page links:
  /// `notes://page/<id>` and the web form `http(s)://<any host>/open/<id>`,
  /// which the hub's bounce page and Android App Links both use. Other
  /// schemes or hosts, a user name or port, an empty id, extra path segments,
  /// and more than one `block` are not page links. An empty `block` is the
  /// same as none.
  static PageLink? fromUri(Uri uri) {
    if (uri.userInfo.isNotEmpty || uri.hasPort) return null;
    final segments = uri.pathSegments;
    final String pageId;
    if (uri.scheme == scheme && uri.host == host) {
      if (segments.length != 1) return null;
      pageId = segments.first;
    } else if ((uri.scheme == 'https' || uri.scheme == 'http') &&
        segments.length == 2 &&
        segments.first == openSegment) {
      pageId = segments.last;
    } else {
      return null;
    }
    if (pageId.isEmpty) return null;
    final blocks = uri.queryParametersAll['block'] ?? const <String>[];
    if (blocks.length > 1) return null;
    final block = blocks.isEmpty || blocks.single.isEmpty
        ? null
        : blocks.single;
    return PageLink(pageId, blockId: block);
  }

  /// The first path segment of the web form, `/open/<id>`.
  static const openSegment = 'open';

  @override
  bool operator ==(Object other) =>
      other is PageLink && other.pageId == pageId && other.blockId == blockId;

  @override
  int get hashCode => Object.hash(pageId, blockId);

  @override
  String toString() => 'PageLink($url)';
}
