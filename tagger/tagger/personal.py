"""Frozen combined mood profiles. Fitting is explicit; inference never learns.

Every mood combines publisher-trained audio evidence with its editable signed
description. Version 1 Horror profiles remain readable for exact rollback. Neither a file name nor a library identity is a prediction feature.
"""
import hashlib
import base64
import json
import os
from pathlib import Path
import re
import threading

import numpy as np
from pydantic import BaseModel, ConfigDict, Field, FiniteFloat, model_validator
from scipy.optimize import minimize
from scipy.special import expit

LABELS = ['Eerie', 'Unnerving', 'Suspenseful', 'Dark', 'Somber', 'Calming', 'Relaxed', 'Uplifting']
WORDS = [['eerie', 'haunting', 'ghostly', 'spectral'], ['unnerving', 'unsettling', 'disturbing', 'sinister'],
         ['suspenseful', 'tense', 'foreboding', 'impending danger'], ['dark', 'ominous', 'dread', 'horror'],
         ['somber', 'desolate', 'bleak', 'funereal'], ['calming', 'peaceful', 'serene', 'reassuring'],
         ['relaxed', 'gentle', 'comfortable', 'intimate'], ['uplifting', 'hopeful', 'inspiring', 'triumphant']]
BOUNDS = [(0, None)] * 4 + [(None, None)] + [(None, 0)] * 3
GRID = [1e-6, 1e-5, 1e-4, .001, .01, .1, 1.]


class WordAxis(BaseModel):
    label: str = Field(max_length=64)
    prompts: list[str] = Field(min_length=1, max_length=16)
    mean: FiniteFloat
    sd: FiniteFloat = Field(gt=1e-9)
    weight: FiniteFloat


class LocalCorrection(BaseModel):
    """Portable normalized audio exemplars; never depend on library/cache IDs."""
    width: FiniteFloat = Field(gt=0, le=1)
    dimensions: int = Field(ge=1, le=4096)
    coefficients: list[FiniteFloat] = Field(min_length=1, max_length=900)
    vectors: str = Field(max_length=20_000_000)

    @model_validator(mode='after')
    def validate_vectors(self):
        try:
            raw = base64.b64decode(self.vectors, validate=True)
        except (ValueError, TypeError) as error:
            raise ValueError('Invalid local audio exemplars') from error
        if len(raw) != len(self.coefficients)*self.dimensions*4:
            raise ValueError('Invalid local audio exemplar dimensions')
        vectors = np.frombuffer(raw,dtype='<f4').reshape((-1,self.dimensions))
        if not np.isfinite(vectors).all() or not np.allclose(np.linalg.norm(vectors,axis=1),1,atol=1e-5):
            raise ValueError('Invalid local audio exemplar normalization')
        return self


def audio_geometry(hidden):
    hidden = np.asarray(hidden,dtype=float)
    if hidden.ndim != 3 or not np.isfinite(hidden).all():
        raise ValueError('Invalid local audio features')
    norms = np.linalg.norm(hidden,axis=2,keepdims=True)
    if (norms < 1e-12).any(): raise ValueError('Empty local audio features')
    # Average layer-wise cosine, keeping all four 1024-dimensional layers.
    geometry = (hidden/norms/np.sqrt(hidden.shape[1])).reshape((len(hidden),-1)).astype('<f4').astype(float)
    return geometry/np.linalg.norm(geometry,axis=1,keepdims=True)


def local_latent(correction, geometry):
    support = np.frombuffer(base64.b64decode(correction.vectors),dtype='<f4').reshape((-1,correction.dimensions)).astype(float)
    if geometry.shape[1] != correction.dimensions: raise ValueError('Incompatible local audio dimensions')
    support /= np.linalg.norm(support,axis=1,keepdims=True)
    kernel = np.exp(-np.maximum(0,1-geometry@support.T)/correction.width)
    return kernel@correction.coefficients


class Profile(BaseModel):
    model_config = ConfigDict(populate_by_name=True)
    version: int = Field(default=1, ge=1, le=8)
    id: str = Field(min_length=1, max_length=128)
    evidence: str = Field(min_length=1, max_length=128)
    created_at: str = Field(alias='createdAt', max_length=64)
    training_signature: str = Field(default='', alias='trainingSignature', max_length=128)
    training_count: int = Field(alias='trainingCount', ge=0, le=500)
    bias: FiniteFloat
    audio_weights: list[FiniteFloat] = Field(alias='audioWeights', min_length=1, max_length=64)
    word_axes: list[WordAxis] = Field(alias='wordAxes', min_length=1, max_length=16)
    method: str | None = Field(default=None, pattern='^(words|local|global)$')
    local: LocalCorrection | None = None
    # Version 8 freezes the accepted foundation; feedback replaces this residual.
    word_version: int | None = Field(default=None, alias='wordVersion', ge=6, le=8)
    adjustment: LocalCorrection | None = None

    @model_validator(mode='after')
    def validate_method(self):
        if self.version < 8 and (self.word_version is not None or self.adjustment is not None):
            raise ValueError('Stable feedback corrections require scoring version 8')
        if self.version < 6 and (self.local is not None or self.method is not None):
            raise ValueError('Local corrections require scoring version 6')
        if self.version >= 6 and ((self.method == 'local') != (self.local is not None) or self.method is None):
            raise ValueError('Invalid learned scoring method')
        return self


class Evidence:
    def __init__(self, metadata, weights):
        self.metadata, self.weights = metadata, weights
        self.id = metadata['id']
        self.layers = metadata['layers']
        self.reference_audio = None
        self.reference_set = None
        self.reference_geometry = None

    def accepts(self, profile):
        return profile.evidence == self.id or (profile.version == 1 and profile.evidence in self.metadata.get('compatibleEvidence', []))

    @classmethod
    def load(cls, root):
        root = Path(root)
        metadata = json.loads((root / 'audio-evidence.json').read_text())
        path = root / 'audio-evidence.npz'
        if hashlib.sha256(path.read_bytes()).hexdigest() != metadata['weightsSha256']:
            raise ValueError('Audio evidence weights do not match their version')
        with np.load(path, allow_pickle=False) as z:
            weights = {k: z[k].copy() for k in z.files}
        evidence = cls(metadata, weights)
        anchors = root / 'semantic-anchors.npz'
        if anchors.is_file():
            with np.load(anchors, allow_pickle=False) as z:
                reference = root / 'reference.npz'
                if str(z['evidence']) != evidence.id or not reference.is_file() or str(z['reference_sha256']) != hashlib.sha256(reference.read_bytes()).hexdigest():
                    raise ValueError('Semantic reference anchors do not match their evidence/reference version')
                audio = np.asarray(z['audio'],dtype=float)
                if audio.ndim != 2 or audio.shape[1] != len(metadata['heads']) or not np.isfinite(audio).all():
                    raise ValueError('Invalid semantic reference anchors')
                evidence.reference_audio = audio
                evidence.reference_set = str(z['reference_set'])
                if 'geometry' in z:
                    geometry = np.asarray(z['geometry'],dtype=float)
                    if geometry.shape != (len(audio),len(evidence.layers)*1024) or not np.isfinite(geometry).all() or not np.allclose(np.linalg.norm(geometry,axis=1),1,atol=1e-5):
                        raise ValueError('Invalid fixed reference audio geometry')
                    evidence.reference_geometry = geometry/np.linalg.norm(geometry,axis=1,keepdims=True)
        return evidence

    def features(self, hidden):
        hidden = np.asarray(hidden, dtype=float)
        if hidden.ndim != 3 or hidden.shape[1:] != (len(self.layers), 1024) or not np.isfinite(hidden).all():
            raise ValueError('Invalid cached audio features')
        norms = np.linalg.norm(hidden, axis=-1, keepdims=True)
        if (norms < 1e-12).any(): raise ValueError('Empty cached audio features')
        H = hidden / norms
        output = []
        for head in self.metadata['heads']:
            members = []
            for member in head['members']:
                A, B = member['calibration']
                raw = H[:, self.layers.index(member['layer'])] @ self.weights[member['coefficient']] + member['intercept']
                members.append(expit(A * raw + B))
            output.append(np.mean(members, axis=0))
        return np.column_stack(output)


class FeatureCache:
    """Versioned pooled hidden features; separate from text-search embeddings."""
    def __init__(self, root, evidence):
        self.root = Path(root) / 'audio-features-v1'
        self.root.mkdir(parents=True, exist_ok=True)
        self.evidence = evidence

    def path(self, content_hash):
        if not re.fullmatch(r'[A-Za-z0-9_-]+', content_hash): raise ValueError('Invalid content hash')
        return self.root / f'{content_hash}.npz'

    def get(self, content_hash):
        path = self.path(content_hash)
        if not path.is_file(): return None
        try:
            with np.load(path, allow_pickle=False) as z:
                if str(z['revision']) != self.evidence.metadata['modelRevision']: return None
                h = z['hidden'].copy()
            if h.shape != (4, 1024) or not np.isfinite(h).all(): return None
            return h
        except (ValueError, OSError, KeyError): return None

    def put(self, content_hash, hidden):
        hidden = np.asarray(hidden, dtype=np.float32)
        if hidden.shape != (4, 1024) or not np.isfinite(hidden).all(): raise ValueError('Invalid hidden features')
        path = self.path(content_hash)
        tmp = path.with_name(f'{content_hash}.{os.getpid()}.{threading.get_ident()}.tmp')
        try:
            with open(tmp, 'wb') as stream:
                np.savez_compressed(stream, hidden=hidden, revision=self.evidence.metadata['modelRevision'])
            os.replace(tmp, path)
        finally:
            tmp.unlink(missing_ok=True)


def profile_scores(profile, evidence, audio, text_raw, geometry=None):
    accepts = evidence.accepts(profile) if hasattr(evidence, 'accepts') else profile.evidence == evidence.id
    if not accepts: raise ValueError('This profile needs another audio evidence version')
    raw = np.asarray(text_raw, dtype=float)
    if raw.shape != (len(audio), len(profile.word_axes)) or not np.isfinite(raw).all(): raise ValueError('Invalid text evidence')
    standardized = word_features(raw,[a.mean for a in profile.word_axes],[a.sd for a in profile.word_axes],
        [a.weight for a in profile.word_axes],penalty_only=(profile.word_version or profile.version)>=7)
    audio = np.asarray(audio)
    if profile.version == 1: audio = audio[:, :8]
    if audio.shape[1] != len(profile.audio_weights): raise ValueError('Incompatible audio evidence dimensions')
    latent = profile.bias + audio @ profile.audio_weights + standardized @ [a.weight for a in profile.word_axes]
    if profile.local is not None:
        if geometry is None: raise ValueError('Local scoring needs audio geometry')
        latent += local_latent(profile.local,geometry)
    if profile.adjustment is not None:
        if geometry is None: raise ValueError('Feedback scoring needs audio geometry')
        latent += local_latent(profile.adjustment,geometry)
    return smooth_score(latent) if profile.version >= 2 else np.clip(latent, 0, 1)


def smooth_score(latent):
    """Strictly monotonic, bounded mapping without a flat clipping plateau."""
    return .5 + np.arctan(latent) / np.pi


def word_features(raw, means, deviations, signs, *, penalty_only=False):
    """Negative descriptions penalize positive evidence, never reward its absence.

    Versions 1–6 retain their signed, centered axes for exact saved inference.
    """
    standardized = (np.asarray(raw,dtype=float)-np.asarray(means))/np.asarray(deviations)
    if penalty_only:
        negative = np.asarray(signs)<0
        standardized[:,negative] = np.maximum(0,standardized[:,negative])
    return standardized


def description_axes(lines):
    """One complete description per line; a leading minus is negative evidence.

    Commas stay inside a phrase. No mood-specific vocabulary or sign rules.
    """
    if not 1 <= len(lines) <= 16 or any(len(line) > 300 for line in lines):
        raise ValueError('Use 1–16 description lines, at most 300 characters each')
    axes = []
    for line in lines:
        negative = line.startswith('- ')
        text = line[2:].strip() if negative else line.removeprefix('+ ').strip()
        if text: axes.append((text, -1 if negative else 1))
    if not axes or not any(sign > 0 for _, sign in axes):
        raise ValueError('Include at least one positive description')
    return axes


def fit_general(features, flags, signs, audio_count, *, validate=True, comparisons=(), diagnostics=None):
    """Fit soft example preferences around an editable semantic starting point.

    Audio weights can move either way for every mood. Description weights keep
    their explicit sign. With little feedback, stronger shrinkage retains the
    description. Nested exclusion estimates generalization, never a guarantee.
    """
    X = np.column_stack([np.ones(len(features)), features])
    scale = np.pi * 1.8 / 4
    positive = max(1, sum(s > 0 for s in signs)); negative = max(1, sum(s < 0 for s in signs))
    base = np.r_[-.674*scale, np.zeros(audio_count), [scale*s/(positive if s > 0 else negative) for s in signs]]
    bounds = [(-8., 8.)] + [(-8., 8.)]*audio_count + [(0., 8.) if s > 0 else (-8., 0.) for s in signs]
    memo = {}
    def train(rows, strength):
        key = (tuple(rows), strength)
        if key in memo: return memo[key]
        # Excluding a rated song also excludes comparisons involving that song.
        omitted = {f.index for i,f in enumerate(flags) if i not in rows}
        pairs = [(high,low) for high,low in comparisons if high not in omitted and low not in omitted]
        if not rows and not pairs: return base, smooth_score(X @ base)
        A = X[[flags[i].index for i in rows]]
        lo = np.array([0. if flags[i].lo is None else flags[i].lo for i in rows])
        hi = np.array([1. if flags[i].hi is None else flags[i].hi for i in rows])
        targets = np.array([np.nan if flags[i].target is None else flags[i].target for i in rows])
        weight = np.array([flags[i].weight for i in rows], dtype=float); weight /= max(1., weight.sum())
        D = np.array([X[high]-X[low] for high,low in pairs]).reshape((-1,X.shape[1]))
        # Text and audio are regularized toward the description, rather than
        # toward an arbitrary all-zero scorer. Focus is deliberately irrelevant:
        # selecting/playing a row must not change a saved fitting problem.
        def objective(w):
            latent = A @ w; pred = smooth_score(latent)
            err = np.minimum(pred-lo, 0) + np.maximum(pred-hi, 0)
            err = np.where(np.isnan(targets), err, pred-targets)
            delta = w-base
            penalty = np.r_[.1, np.ones(len(w)-1)]
            # Smooth ordering preference, no hard minimum score gap. Bounded
            # coefficients and the semantic prior prevent unbounded separation.
            pair_delta = D @ w
            pair_cost = .02*float(np.mean(np.logaddexp(0., -pair_delta))) if len(D) else 0.
            pair_grad = -.02*D.T @ (1/(1+np.exp(np.clip(pair_delta,-50,50))))/len(D) if len(D) else 0.
            return float(np.sum(weight*err**2) + pair_cost + strength*np.sum(penalty*delta**2)), 2*A.T @ (weight*err/(np.pi*(1+latent**2))) + pair_grad + 2*strength*penalty*delta
        result = minimize(objective, base, jac=True, bounds=bounds, method='L-BFGS-B', options={'maxiter':1500, 'ftol':1e-12, 'gtol':1e-8})
        if not result.success:
            result = minimize(objective, result.x, jac=True, bounds=bounds, method='SLSQP', options={'maxiter':1500, 'ftol':1e-11})
        if not result.success: raise ValueError('The update did not converge; the saved scorer is unchanged')
        memo[key] = result.x, smooth_score(X @ result.x)
        return memo[key]
    def miss(v, f): return abs(v-f.target) if f.target is not None else max(0, (f.lo or 0)-v) + max(0, v-(1 if f.hi is None else f.hi))
    grid = [.00001, .0001, .001, .01, .1]
    def choose(rows):
        if not validate: return .001
        if len(rows) < 5: return .01
        return min((np.mean([miss(train([j for j in rows if j != held], strength)[1][flags[held].index], flags[held]) for held in rows]), -strength, strength) for strength in grid)[2]
    rows = list(range(len(flags))); held = 0; held_errors = []
    if validate and len(rows) >= 5:
        for i in rows:
            others = [j for j in rows if j != i]
            error = miss(train(others, choose(others))[1][flags[i].index], flags[i])
            held_errors.append(error)
            held += error < 1e-7
    weights, scores = train(rows, choose(rows))
    if diagnostics is not None:
        diagnostics['validation_mae'] = float(np.mean(held_errors)) if held_errors else None
    return weights, scores, (int(held), len(rows) if validate and len(rows) >= 5 else 0)


def directional_losses(latent, flags):
    """Directional judgments reward agreement continuously, never a point target.

    Numeric overrides remain numeric. Historical explicit ceilings/floors retain
    their original meaning rather than being relabeled as membership judgments.
    """
    latent = np.asarray(latent, dtype=float)
    values = smooth_score(latent)
    derivative = 1/(np.pi*(1+latent**2))
    losses = np.zeros(len(flags)); gradients = np.zeros(len(flags))
    for i,f in enumerate(flags):
        polarity,pivot = f.polarity,f.pivot
        if polarity is None and f.target is None:
            if f.lo is None and f.hi is not None: polarity,pivot = -1,f.hi
            elif f.hi is None and f.lo is not None: polarity,pivot = 1,f.lo
        if polarity is not None:
            offset = np.tan(np.pi*(np.clip(pivot, .00001, .99999)-.5))
            z = latent[i]-offset
            p = float(np.clip(smooth_score(z), 1e-12, 1-1e-12))
            y = (polarity+1)/2
            losses[i] = -y*np.log(p)-(1-y)*np.log1p(-p)
            gradients[i] = (p-y)/(p*(1-p)*np.pi*(1+z*z))
        elif f.target is not None:
            losses[i] = (values[i]-f.target)**2
            gradients[i] = 2*(values[i]-f.target)*derivative[i]
        else:
            error = min(values[i]-(f.lo or 0),0)+max(values[i]-(1 if f.hi is None else f.hi),0)
            losses[i] = error**2
            gradients[i] = 2*error*derivative[i]
    return losses, gradients


def fit_directional(features, flags, signs, audio_count, *, validate=True, comparisons=(), diagnostics=None, reference_features=None, fixed_regularization=None):
    """Membership direction + conviction, with the same frozen score mapping.

    Definitely/No are stronger evidence, not .9/.1 point targets. Held-out
    comparisons never include the held recording. Near-tied validation choices
    favor retaining explicit feedback rather than stronger semantic shrinkage.
    """
    X = np.column_stack([np.ones(len(features)),features])
    anchored = reference_features is not None
    R = np.column_stack([np.ones(len(reference_features)),reference_features]) if anchored else None
    if anchored and (len(R) < 2 or R.shape[1] != X.shape[1] or not np.isfinite(R).all()):
        raise ValueError('Invalid fixed reference features')
    scale = np.pi*1.8/4
    base = np.r_[-.674*scale,np.zeros(audio_count),[scale*s/max(1,signs.count(s)) for s in signs]]
    bounds = [(-8.,8.)]*(audio_count+1)+[(0.,8.) if s>0 else (-8.,0.) for s in signs]
    penalty = np.ones(len(base)) if anchored else np.r_[.1,np.ones(len(base)-1)]
    memo = {}
    def train(rows, strength):
        key = (tuple(rows),strength)
        if key in memo: return memo[key]
        omitted = {f.index for i,f in enumerate(flags) if i not in rows}
        pairs = [(a,b) for a,b in comparisons if a not in omitted and b not in omitted]
        if not rows and not pairs: return base,smooth_score(X@base)
        fs = [flags[i] for i in rows]; A = X[[f.index for f in fs]]
        weight = np.array([f.weight*f.confidence for f in fs],dtype=float); weight /= 4 if anchored else max(1.,4*len(fs))
        # Selected feedback is not a prevalence sample. Without both yes and no
        # membership examples, retain a stronger semantic reference prior.
        directions = {f.polarity for f in fs if f.pivot == .5 and f.polarity is not None}
        anchor_strength = .03 if directions == {-1,1} else .3
        D = np.array([X[a]-X[b] for a,b in pairs]).reshape((-1,X.shape[1]))
        def objective(w):
            losses,gradient = directional_losses(A@w,fs)
            delta = w-base; pair_delta = D@w
            reference_delta = R@delta if anchored else None
            reference_cost = anchor_strength*float(np.mean(reference_delta**2)) if anchored else 0.
            reference_gradient = 2*anchor_strength*R.T@reference_delta/len(R) if anchored else 0.
            pair_cost = .02*float(np.mean(np.logaddexp(0.,-pair_delta))) if len(D) else 0.
            pair_gradient = -.02*D.T@(1/(1+np.exp(np.clip(pair_delta,-50,50))))/len(D) if len(D) else 0.
            return float(weight@losses+pair_cost+reference_cost+strength*np.sum(penalty*delta**2)), A.T@(weight*gradient)+pair_gradient+reference_gradient+2*strength*penalty*delta
        result = minimize(objective,base,jac=True,bounds=bounds,method='L-BFGS-B',options={'maxiter':2000,'ftol':1e-12,'gtol':1e-8})
        if not result.success:
            result = minimize(objective,result.x,jac=True,bounds=bounds,method='SLSQP',options={'maxiter':2000,'ftol':1e-11})
        if not result.success: raise ValueError('The feedback fit did not converge; saved scoring is unchanged')
        memo[key] = result.x,smooth_score(X@result.x)
        return memo[key]
    def loss_at(weights,i):
        return directional_losses([X[flags[i].index]@weights],[flags[i]])[0][0]
    grid = [.00001,.0001,.001,.01,.1]
    def choose(rows):
        if fixed_regularization is not None: return fixed_regularization
        if anchored: return .0001  # fixed policy; no noisy hyperparameter winner
        if not validate: return .0001
        if len(rows)<5: return .001
        errors = {strength:np.array([loss_at(train([j for j in rows if j!=i],strength)[0],i) for i in rows]) for strength in grid}
        weight = np.array([flags[i].confidence*flags[i].weight for i in rows],dtype=float); weight /= weight.sum()
        best = min(grid,key=lambda s:float(weight@errors[s]))
        eligible = []
        for strength in grid:
            difference = errors[strength]-errors[best]
            mean = float(weight@difference)
            # Paired standard error, using the effective number of judgments.
            variance = float(weight@((difference-mean)**2))
            se = np.sqrt(variance*float(weight@weight))
            if mean <= se+1e-12: eligible.append(strength)
        return min(eligible)
    rows = list(range(len(flags))); held_errors = []
    if validate and len(rows)>=5:
        for i in rows:
            others = [j for j in rows if j!=i]
            held_errors.append(loss_at(train(others,choose(others))[0],i))
    strength = choose(rows)
    weights,scores = train(rows,strength)
    if diagnostics is not None:
        diagnostics['validation_loss'] = float(np.average(held_errors,weights=[f.confidence*f.weight for f in flags])) if held_errors else None
        diagnostics['regularization'] = strength
    return weights,scores,(0,0)


def fit(features, flags):
    """Nested exclusion chooses regularization without using the held judgment.

    Only explicit fit calls read flags. Fresh/stale weights are honored, and
    contradictory labels return a review with misses rather than being rewritten.
    """
    if len(flags) < 5: raise ValueError('Flag at least five analyzed songs before learning')
    if not any(f.lo is not None and f.lo >= .75 for f in flags) or not any(f.hi is not None and f.hi <= .75 for f in flags):
        raise ValueError('Include examples that belong and examples that should not match before learning')
    X = np.column_stack([np.ones(len(features)), features]); memo = {}
    def train(rows, regularization):
        key = (tuple(rows), regularization)
        if key in memo: return memo[key]
        idx = [flags[i].index for i in rows]; A = X[idx]
        lo = np.array([0 if flags[i].lo is None else min(1, flags[i].lo + .03) for i in rows])
        hi = np.array([1 if flags[i].hi is None else max(0, flags[i].hi - .03) for i in rows])
        weight = np.array([flags[i].weight for i in rows]); weight /= weight.sum()
        def objective(w):
            pred = A @ w; err = np.minimum(pred - lo, 0) + np.maximum(pred - hi, 0)
            cost = (weight * err**2).sum() + regularization * (w[1:] @ w[1:] + .1*w[0]**2)
            grad = 2*A.T @ (weight*err) + 2*regularization*np.r_[.1*w[0], w[1:]]
            return float(cost), grad
        start = np.zeros(X.shape[1]); start[0] = .2
        bounds = [(-3, 3)] + BOUNDS * 2
        result = minimize(objective, start, jac=True, bounds=bounds, method='L-BFGS-B', options={'maxiter':1500, 'ftol':1e-12, 'gtol':1e-8})
        if not result.success:
            result = minimize(objective, result.x, jac=True, bounds=bounds, method='SLSQP', options={'maxiter':1500, 'ftol':1e-11})
        if not result.success: raise ValueError('Learning could not converge; keep the current scorer')
        memo[key] = result.x, np.clip(X @ result.x, 0, 1)
        return memo[key]
    def miss(v, flag): return max(0, (flag.lo or 0)-v) + max(0, v-(1 if flag.hi is None else flag.hi))
    def choose(rows):
        ranking = []
        for strength in GRID:
            errors = [miss(train([j for j in rows if j != held], strength)[1][flags[held].index], flags[held]) for held in rows]
            ranking.append((float(np.mean(errors)), -strength, strength))
        return min(ranking)[2]
    all_rows = list(range(len(flags))); held = 0
    for i in all_rows:
        rows = [j for j in all_rows if j != i]
        v = train(rows, choose(rows))[1][flags[i].index]
        held += miss(v, flags[i]) < 1e-7
    coefficients, scores = train(all_rows, choose(all_rows))
    return coefficients, scores, (int(held), len(flags))
