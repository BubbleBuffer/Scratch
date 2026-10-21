import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

void main() {
  test('legacy split labels are not shown as human decisions', () {
    expect(
      outlineDecision(['No context boundary selected from 4 candidates']),
      isNull,
    );
    expect(
      outlineDecision(['Context boundary selected from 2 candidates']),
      isNull,
    );
    expect(
      outlineDecision(['Ship for Chrome first.']),
      'Ship for Chrome first.',
    );
  });
  test('a two to six word prefix becomes a name and the remainder a gist', () {
    expect(outlineNameGist('Two words: the rest: stays intact', 'Summary.'), (
      name: 'Two words',
      gist: 'the rest: stays intact',
    ));
    expect(outlineNameGist('One: details', 'First sentence. Second.'), (
      name: 'One: details',
      gist: 'First sentence.',
    ));
    expect(
      outlineNameGist(
        'one two three four five six seven: details',
        'Gist! More.',
      ),
      (name: 'one two three four five six seven: details', gist: 'Gist!'),
    );
    expect(outlineNameGist('one two three four five six: details', ''), (
      name: 'one two three four five six',
      gist: 'details',
    ));
    expect(outlineNameGist('Two words:no space', ''), (
      name: 'Two words:no space',
      gist: '',
    ));
  });
  test('section names drop an exact leading chapter name and separator', () {
    for (final separator in [': ', ' › ', ' — ', ' – ', ' - ', ' / ']) {
      expect(
        outlineNameGist(
          'Headless cursor${separator}Mouse Keys: fixed',
          '',
          chapterName: 'Headless cursor',
        ),
        (name: 'Mouse Keys', gist: 'fixed'),
      );
    }
    expect(
      outlineNameGist(
        'Headless cursor support: fixed',
        '',
        chapterName: 'Headless cursor',
      ).name,
      'Headless cursor support',
    );
  });
  test('BubblePlay sentence titles use short names without changing raw text', () {
    expect(
      outlineNameGist(
        'BubblePlay headless cursor: Mouse Keys workaround hardened after virtual-mouse driver route dropped',
        '',
      ),
      (
        name: 'BubblePlay headless cursor',
        gist: 'Mouse Keys workaround hardened after virtual-mouse driver route dropped',
      ),
    );
    expect(
      outlineNameGist(
        'Native virtual-file drag-and-drop for web→PC: implemented, deployed, and extended with host hover preview',
        '',
      ),
      (
        name: 'Native virtual-file drag-and-drop for web→PC',
        gist: 'implemented, deployed, and extended with host hover preview',
      ),
    );
    const raw =
        'BubblePlay headless cursor: Mouse Keys fix, alternatives, virtual mouse driver';
    expect(outlineNameGist(raw, ''), (
      name: 'BubblePlay headless cursor',
      gist: 'Mouse Keys fix, alternatives, virtual mouse driver',
    ));
    expect(
      outlineNameGist(
        raw,
        'Summary.',
        chapterName: 'BubblePlay headless cursor',
      ),
      (
        name: 'Mouse Keys fix, alternatives, virtual mouse driver',
        gist: 'Summary.',
      ),
    );
    expect(
      outlineNameGist(
        'Native virtual-file drop: Chrome live drops pass, Explorer still blocked',
        '',
      ),
      (
        name: 'Native virtual-file drop',
        gist: 'Chrome live drops pass, Explorer still blocked',
      ),
    );
    const long =
        'Release-readiness audit, Go/TOTP plan, outage recovery, large-drop fix';
    expect(outlineNameGist(long, 'Audit complete. Recovery tested.'), (
      name: long,
      gist: 'Audit complete.',
    ));
  });
}
