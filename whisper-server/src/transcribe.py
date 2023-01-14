# import numpy
import json
import re

import whisper

MODEL = None
ALPHANUMERIC_REGEX = re.compile(r'[^\w\s]')


def load_model(path):
    global MODEL
    MODEL = whisper.load_model(path, in_memory=True)


def transcribe(audio):
    """
    Args:
        audio: path to file
    """
    # With some futzing this can probably turn into
    #
    # audio = numpy.array(audio, dtype=numpy.float32)
    # audio = whisper.pad_or_trim(audio)
    # mel = whisper.log_mel_spectrogram(audio).to(model.device)
    # options = whisper.DecodingOptions(fp16=False)
    # result = whisper.decode(model, mel, options)
    #
    # This will stop it using ffmpeg since the bot can resample the audio
    # before sending it.
    response = {
        "channel": {"alternatives": [{"transcript": ""}]}
    }
    try:
        result = whisper.transcribe(MODEL, audio, verbose=None, fp16=False)
        cleaned_text = ALPHANUMERIC_REGEX.sub('', result["text"].lower()).strip()
        response["channel"]["alternatives"][0]["transcript"] = cleaned_text
    except Exception as e:
        print(e)

    return json.dumps(response)


def transcribe_raw(audio):
    """
    Args:
        audio: a numpy array of 16khz mono audio in F32LE format
    """
    response = {
        "channel": {"alternatives": [{"transcript": ""}]}
    }
    try:
        audio = whisper.pad_or_trim(audio)
        mel = whisper.log_mel_spectrogram(audio).to(MODEL.device)
        options = whisper.DecodingOptions(fp16=False)
        result = whisper.decode(MODEL, mel, options)
        cleaned_text = ALPHANUMERIC_REGEX.sub('', result["text"].lower()).strip()
        response["channel"]["alternatives"][0]["transcript"] = cleaned_text
    except Exception as e:
        print(e)

    return json.dumps(response)
