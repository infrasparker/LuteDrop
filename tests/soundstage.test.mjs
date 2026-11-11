import { test } from 'node:test';
import assert from 'node:assert/strict';
import { trackDestinations, sceneMatches, playbackLabel, viewTrackIds, captureSceneTracks, trackLocations, trackSearchText } from '../src/utils/soundstage.ts';
import { sceneStatus, isBoardActive } from '../src/shared/playback.ts';
const state = (id, enabled = true, clip = 'rain', volume = .4) => ({trackId:id,enabled,selectedClipId:clip,volume});
test('scene changes detect selected clips and volumes without flagging unused tracks', () => {
  assert.equal(sceneMatches([state('weather')],[state('weather'),state('music',false)]),true);
  assert.equal(sceneMatches([state('weather')],[state('weather',true,'storm')]),false);
  assert.equal(sceneMatches([state('weather')],[state('weather',true,'rain',.6)]),false);
  assert.equal(sceneMatches([state('weather')],[state('weather'),state('music')]),false);
  assert.equal(sceneMatches([state('weather')],[state('weather',false)]),false);
});
test('scene status ignores disabled references and uses reusable track IDs', () => {
  const scene={tracks:[state('weather'),state('music',false)]};
  const states=new Map([['weather',{isPlaying:true,isPaused:false}]]);
  assert.equal(sceneStatus(scene,states),'active');
  assert.equal(isBoardActive({trackIds:['weather']},states),true);
  states.set('weather',{isPlaying:false,isPaused:true});
  assert.equal(sceneStatus(scene,states),'paused');
});
test('playback labels describe manual alternatives and random gaps', () => {
  assert.equal(playbackLabel({type:'seamless'}),'Seamless');
  assert.equal(playbackLabel({type:'oneShot'}),'One shot');
  assert.equal(playbackLabel({type:'shuffle',repeat:true,transition:{type:'gap',min:5,max:20}}),'Shuffle · 5–20s gaps · Repeat');
});

test('scene layouts include optional cards but exclude unrelated biome tracks', () => {
  const board={trackIds:['rain','forest','desert'],scenes:[{id:'woods',tracks:[state('rain',false),state('forest')]}]};
  assert.deepEqual(viewTrackIds(board,'woods'),['rain','forest']);
  assert.deepEqual(viewTrackIds(board,null),['rain','forest','desert']);
  assert.deepEqual(viewTrackIds(board,'deleted'),[]);
});
test('saving a scene never adds other board cards and All snapshots only active tracks', () => {
  const board={trackIds:['weather','music','desert'],scenes:[{id:'woods',tracks:[state('weather',false,'rain'),state('music',true,null)]}]};
  const tracks=board.trackIds.map((id) => ({id,volume:.4,clips:[{id:'rain'}],mode:{type:id==='music'?'shuffle':'seamless'}}));
  const playback=new Map([['music',{isPlaying:true,currentClipId:'random-choice'}],['desert',{isPlaying:true,currentClipId:'rain'}]]);
  const saved=captureSceneTracks(board,'woods',tracks,[],playback);
  assert.deepEqual(saved.map((s) => s.trackId),['weather','music']);
  assert.equal(saved[0].enabled,false);
  assert.equal(saved[1].selectedClipId,null,'saving a shuffled scene must not freeze the current random pick');
  assert.deepEqual(captureSceneTracks(board,null,tracks,[],playback).map((s) => s.trackId),['music','desert']);
});

test('same-named inventory tracks can be distinguished and searched by their usage and clips', () => {
  const boards=[{name:'Travel',trackIds:['journey','unused-here'],scenes:[{name:'Forest',tracks:[state('journey')]}]},
    {name:'City',trackIds:['tavern'],scenes:[{name:'Inn',tracks:[state('tavern')]}]}];
  const journey={id:'journey',name:'Music',clips:[{name:"The\u00a0Hunter's\u00a0Path"}]};
  const tavern={id:'tavern',name:'Music',clips:[{name:'Bard Dance'}]};
  assert.deepEqual(trackLocations('journey',boards),['Travel / Forest']);
  assert.deepEqual(trackLocations('unused-here',boards),['Travel / All tracks']);
  assert.deepEqual(trackLocations('other-configuration',boards),[]);
  assert.equal(trackSearchText(journey,boards).includes('travel'),true);
  assert.equal(trackSearchText(tavern,boards).includes('travel'),false);
  assert.equal(trackSearchText(journey,boards).includes("hunter's path"),true);
});


test('sharing destinations include same-board scenes even when already present but disabled', () => {
  const boards = [
    { id: 'village', name: 'Village', trackIds: [], scenes: [{ id: 'tavern', name: 'Tavern', tracks: [] }] },
    { id: 'travel', name: 'Travel', trackIds: ['horse'], scenes: [
      { id: 'forest', name: 'Forest', tracks: [state('horse', false)] },
      { id: 'grass', name: 'Grassland', tracks: [] }] }
  ];
  const before = structuredClone(boards);
  const scenes = trackDestinations(boards, 'horse', 'travel');
  assert.deepEqual(scenes.map((item) => item.id), ['forest', 'grass', 'tavern']);
  assert.equal(scenes[0].added, true);
  assert.equal(scenes[0].props.disabled, true);
  assert.equal(scenes[1].props.disabled, false);
  assert.equal(scenes[0].name, 'Travel / Forest');
  const destinations = trackDestinations(boards, 'horse', 'travel', 'boards');
  assert.deepEqual(destinations.map((item) => [item.id, item.added]), [['travel', true], ['village', false]]);
  assert.deepEqual(boards, before, 'listing destinations must preserve custom board and scene order');
  assert.deepEqual(trackDestinations([], 'horse'), []);
});
