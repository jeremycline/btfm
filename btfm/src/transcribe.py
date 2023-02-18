from typing import Union

import numpy as np
import torch
import whisper

MODEL = None


def load_model(path):
    global MODEL
    MODEL = whisper.load_model(path)


def transcribe(audio: Union[str, np.ndarray, torch.Tensor], fp16=False):
    """
    Transcribe using Whisper.

    You must call load_model() before using this.

    audio: The path to audio or either a NumPy array or Tensor containing the
        audio. If it's not a path the audio must be mono f32 16 kHz format.
    fp16: If your GPU supports FP16, you can set this to 'True' for better performance.
    """
    text = ""
    if MODEL is None:
        print("Programmer error, you must load the model first with 'load_model()'")
    else:
        try:
            result = whisper.transcribe(MODEL, audio, verbose=None, fp16=fp16)
            text = result["text"]
        except Exception as e:
            print(e)

    return text
