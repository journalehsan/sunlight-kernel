#!/usr/bin/env python3
"""Analyze finalized QEMU PCM captures; source identification uses ffmpeg and standard Python.
Usage: empires_audio_analyze.py captured.wav [source.wav]
Correlation searches the expected source for a 3s capture envelope after
resampling, allowing an unknown guest/QEMU start offset and volume scaling.
"""
import argparse
from array import array
import json
import math
from pathlib import Path
import wave

def load(path):
    with wave.open(str(path),'rb') as w:
        assert w.getsampwidth()==2 and w.getcomptype()=='NONE'
        data=w.readframes(w.getnframes()); rate=w.getframerate();channels=w.getnchannels()
    samples=array('h');samples.frombytes(data)
    import sys
    if sys.byteorder!='little':samples.byteswap()
    return samples,rate,channels

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('capture',type=Path);p.add_argument('source',type=Path,nargs='?');a=p.parse_args()
    samples,rate,channels=load(a.capture); n=len(samples)
    result=dict(file=a.capture.name,sample_rate=rate,channels=channels,samples=n,seconds=n/channels/rate,
        peak=max(map(abs,samples),default=0),rms=math.sqrt(sum(float(x)*x for x in samples)/max(n,1)),
        clipped_samples=sum(abs(x)>=32767 for x in samples),nonzero_samples=sum(x!=0 for x in samples))
    longest=run=0
    for frame in range(0,n,channels):
        if all(v==0 for v in samples[frame:frame+channels]):run+=1;longest=max(longest,run)
        else:run=0
    result['longest_zero_run_ms']=longest*1000/rate
    result['max_adjacent_sample_delta']=max((abs(samples[i]-samples[i-channels]) for i in range(channels,n)),default=0)
    if a.source:
        import subprocess
        def features(path):
            bands=[]
            for low,high in [(80,400),(400,1400),(1400,3500)]:
                raw=subprocess.check_output(['ffmpeg','-v','error','-i',str(path),'-af',f'highpass=f={low},lowpass=f={high}', '-ar','8000','-ac','1','-f','s16le','-'])
                data=array('h');data.frombytes(raw)
                bands.append([math.sqrt(sum(float(v)*v for v in data[n:n+160])/160) for n in range(0,len(data)-159,160)])
            return bands
        x=features(a.source);y=features(a.capture)
        window=min(150,len(y[0]));assert window>=50
        start=max(range(len(y[0])-window+1),key=lambda n:sum(v*v for v in y[0][n:n+window]))
        segments=[]
        for band in y:
            segment=band[start:start+window];mean=sum(segment)/window
            segment=[v-mean for v in segment];norm=math.sqrt(sum(v*v for v in segment))
            segments.append((segment,norm))
        best=(-1,0)
        for offset in range(len(x[0])-window+1):
            scores=[]
            for band,(segment,norm) in zip(x,segments):
                source=band[offset:offset+window];mean=sum(source)/window
                variance=sum((v-mean)**2 for v in source)
                scores.append(sum((v-mean)*t for v,t in zip(source,segment))/max(math.sqrt(variance)*norm,1))
            score=sum(scores)/len(scores)
            if score>best[0]:best=(score,offset)
        result.update(source=a.source.name,source_envelope_correlation=best[0],source_offset_seconds=best[1]*.02,capture_offset_seconds=start*.02,
            correlation_method='mean normalized RMS-envelope correlation in 80-400/400-1400/1400-3500Hz bands, 20ms bins, energetic <=3s capture window; compare wrong-source controls; no claim of physical listening or glitch-free playback')
    print(json.dumps(result,indent=2))
if __name__=='__main__':main()
