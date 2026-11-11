from tagger.signed_search import candidates, key


def test_recombining_understands_polarity_and_phrase_boundaries():
    current = ['eerie, haunting', '- hopeful, triumphant']
    proposals = candidates(current, [], [], 'arrange')
    assert ['eerie', 'haunting', '- hopeful, triumphant'] in proposals
    assert ['eerie, haunting', '- hopeful', '- triumphant'] in proposals
    assert any('- haunting' in p for p in proposals)
    assert all(any(not line.startswith('- ') for line in p) for p in proposals)
    assert current == ['eerie, haunting', '- hopeful, triumphant']
    for proposal in proposals:
        words = [word.strip().casefold() for line in proposal for word in line.removeprefix('- ').split(',')]
        assert len(words) == len(set(words)), 'A proposal should not repeat a term inside a group and on its own'


def test_suggestions_try_both_signs_and_do_not_repeat_existing_proposals():
    base = ['ominous']
    proposals = candidates(base, [], ['calming'], 'suggest')
    assert ['ominous', 'calming'] in proposals
    assert ['ominous', '- calming'] in proposals
    later = candidates(base, [], ['calming'], 'suggest', previous=proposals)
    assert not {key(p) for p in proposals}.intersection(key(p) for p in later)
    assert not any('calming' in line for p in candidates(base, [], ['calming'], 'suggest', excluded=['calming']) for line in p)
