/// Interim presentation for sentence titles; stored titles and anchors stay raw.
({String name, String gist}) outlineNameGist(
  String title,
  String summary, {
  String? chapterName,
}) {
  var name = title.trim();
  if (chapterName != null && chapterName.isNotEmpty) {
    for (final separator in [': ', ' › ', ' — ', ' – ', ' - ', ' / ']) {
      final prefix = '$chapterName$separator';
      if (name.startsWith(prefix)) {
        name = name.substring(prefix.length).trim();
        break;
      }
    }
  }
  final colon = name.indexOf(': ');
  if (colon > 0) {
    final left = name.substring(0, colon).trim();
    final words = left.split(RegExp(r'\s+')).length;
    final right = name.substring(colon + 2).trim();
    if (words >= 2 && words <= 6 && right.isNotEmpty) {
      return (name: left, gist: right);
    }
  }
  final text = summary.trim();
  final end = text.indexOf(RegExp(r'[.!?](\s|$)'));
  return (name: name, gist: end < 0 ? text : text.substring(0, end + 1));
}

/// Older imported caches lost decision kinds; suppress their known split labels.
String? outlineDecision(Iterable<String> decisions) => decisions
    .map((d) => d.trim())
    .where(
      (d) =>
          d.isNotEmpty &&
          !d.startsWith('No context boundary selected from ') &&
          !d.startsWith('Context boundary selected from '),
    )
    .firstOrNull;
