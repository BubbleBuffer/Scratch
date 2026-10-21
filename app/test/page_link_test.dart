import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/page_link.dart';

void main() {
  const page = '01M5CZCX81NZWZM4KEBKTR9SJN';
  const block = '01M5CZD0Q4ZB3M8Y5E7V2H6K9N';

  test('parses a page link', () {
    final link = PageLink.parse('notes://page/$page');
    expect(link, const PageLink(page));
    expect(link!.blockId, isNull);
  });

  test('parses a block link', () {
    expect(
      PageLink.parse('notes://page/$page?block=$block'),
      const PageLink(page, blockId: block),
    );
  });

  test('ignores surrounding spaces and an empty block', () {
    expect(
      PageLink.parse('  notes://page/$page?block=  '),
      const PageLink(page),
    );
  });

  test('builds the same text the page menu copies', () {
    expect(const PageLink(page).url, 'notes://page/$page');
    expect(
      const PageLink(page, blockId: block).url,
      'notes://page/$page?block=$block',
    );
  });

  test('round-trips through its own text', () {
    const link = PageLink(page, blockId: block);
    expect(PageLink.parse(link.url), link);
  });

  test('reads Uri forms the platform hands over', () {
    expect(
      PageLink.fromUri(Uri.parse('NOTES://PAGE/$page')),
      const PageLink(page),
    );
  });

  test('parses the web form of a page link, from any host', () {
    expect(
      PageLink.parse('https://notes.breadcat.cc/open/$page'),
      const PageLink(page),
    );
    expect(
      PageLink.parse('https://notes.breadcat.cc/open/$page?block=$block'),
      const PageLink(page, blockId: block),
    );
    expect(
      PageLink.parse('http://127.0.0.1/open/$page?block=  '),
      const PageLink(page),
    );
    expect(
      PageLink.parse('https://example.com/open/$page'),
      const PageLink(page),
    );
    expect(
      PageLink.fromUri(Uri.parse('HTTPS://NOTES.BREADCAT.CC/open/$page')),
      const PageLink(page),
    );
  });

  test('the web form still builds the notes link the app opens', () {
    expect(
      PageLink.parse('https://notes.breadcat.cc/open/$page?block=$block')?.url,
      'notes://page/$page?block=$block',
    );
  });

  for (final text in [
    'notes://auth?code=one&state=two',
    'notes://pair?hub=http://100.101.102.103:47100&token=abc',
    'https://example.com/notes://page/$page',
    'http://127.0.0.1:47100/open/$page',
    'https://notes.breadcat.cc/open/$page/',
    'https://notes.breadcat.cc/open/$page/extra',
    'https://notes.breadcat.cc/open/',
    'https://notes.breadcat.cc/open',
    'https://notes.breadcat.cc/$page',
    'https://notes.breadcat.cc/view/$page',
    'https://notes.breadcat.cc:8443/open/$page',
    'https://user@notes.breadcat.cc/open/$page',
    'https://notes.breadcat.cc/open/$page?block=$block&block=$block',
    'ftp://notes.breadcat.cc/open/$page',
    'notes://open/$page',
    'notes://page',
    'notes://page/',
    'notes://page/$page/extra',
    'notes://page/$page?block=$block&block=$block',
    'notes://page:8080/$page',
    'notes://user@page/$page',
    'notes:page/$page',
    'notes://page?block=$block',
    page,
    '',
    'not a link',
  ]) {
    test('rejects $text', () {
      expect(PageLink.parse(text), isNull);
    });
  }
}
