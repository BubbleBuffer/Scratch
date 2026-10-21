// Formatting helpers shared by screens.

import 'dart:math' as math;

import '../core/clock.dart';

String relativeTime(int ms) {
  final t = DateTime.fromMillisecondsSinceEpoch(ms);
  final now = clockNow();
  final d = now.difference(t);
  if (d.inSeconds < 60) return 'Just now';
  if (d.inMinutes < 60) return '${d.inMinutes}m ago';
  if (d.inHours < 24) return '${d.inHours}h ago';
  if (d.inHours < 48) return 'Yesterday';
  if (d.inDays < 7) return '${d.inDays}d ago';
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  final md = '${months[t.month - 1]} ${t.day}';
  return t.year == now.year ? md : '$md, ${t.year}';
}

/// 1234 -> "1,234".
String formatCount(int n) => n >= 1000 ? '${formatCount(n ~/ 1000)},${(n % 1000).toString().padLeft(3, '0')}' : '$n';

/// A page's display name: its title, then its first body line, then "Untitled".
String displayTitle(String title, {String firstLine = ''}) {
  if (title.trim().isNotEmpty) return title;
  final line = firstLine.trim();
  return line.isNotEmpty ? line : 'Untitled';
}

/// "1 task", "2 tasks"; [many] for irregular plurals.
String plural(int n, String one, [String? many]) => '${formatCount(n)} ${n == 1 ? one : (many ?? '${one}s')}';

double clampD(double v, double lo, double hi) => math.max(lo, math.min(hi, v));
