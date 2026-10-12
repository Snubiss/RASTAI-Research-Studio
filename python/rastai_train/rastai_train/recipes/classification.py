"""The classification recipe: cross-entropy on a classifier head (layout python.md sections 5.1 and 5.9). TextCNN is
its first model; ViT and encoder-plus-head designs use it too (M6).

The rows are `{"text": ..., "label": ...}` (an `id` names a row for its split). prepare() builds the word vocabulary
from the training texts, at most the design's vocab_size words, and the label list from every row, sorted, so the
same rows give the same ids on every machine and in every resumed job. A batch is padded with PADDING_ID, or cut, to
the design's sequence_length, so every forward pass has the shape the design was planned with.

The memory defaults of the spike (answer-only loss, gradient checkpointing, LoRA masters in float32) are for
decoder fine-tuning and join with the SFT recipe (M5); none applies to a classifier this size.
"""

import torch
from rastai_data.word_vocabulary import PADDING_ID
from rastai_formats.generated.model import (
    GraphNode,
    ModelSpec,
    classifier_head_attributes_from_json,
    embedding_attributes_from_json,
)
from rastai_plugin.catalog.generated import RRS_T0003, RRS_T0005, RRS_T2005
from torch.nn import functional

from rastai_data import WordVocabulary, build_word_vocabulary, encode_text
from rastai_plugin import Batch, Recipe, StepResult, TrainingContext
from rastai_train.errors import TrainError

RECIPE_NAME = "classification"
TEXT_COLUMN = "text"
LABEL_COLUMN = "label"
ACCURACY_METRIC = "classification.accuracy"
# recipe_state's key: the label list, saved so a resume can't silently map labels to other classes.
LABELS_KEY = "labels"
# The ops this recipe needs at the two ends of the design.
EMBEDDING_OP = "embedding"
CLASSIFIER_HEAD_OP = "classifier_head"


class ClassificationRecipe(Recipe):
    """Cross-entropy between the classifier head's logits and each row's label."""

    name = RECIPE_NAME

    def prepare(self, context: TrainingContext) -> None:
        spec = context.model.spec
        training_rows = context.loader.examples
        check_rows(training_rows + context.validation_examples)
        self.model = context.model
        self.device = context.device
        self.sequence_length = spec.defaults.sequence_length
        self.vocabulary = build_word_vocabulary(row_texts(training_rows), vocabulary_size(spec))
        self.labels = sorted(set(row_labels(training_rows + context.validation_examples)))
        check_label_count(self.labels, class_count(spec))
        self.label_ids = {label: label_id for label_id, label in enumerate(self.labels)}

    def training_step(self, batch: Batch) -> StepResult:
        token_ids = encode_batch(self.vocabulary, batch.examples, self.sequence_length).to(self.device)
        label_ids = torch.tensor([self.label_ids[row[LABEL_COLUMN]] for row in batch.examples], device=self.device)
        logits = self.model(token_ids)
        loss = functional.cross_entropy(logits, label_ids)
        accuracy = (logits.argmax(dim=-1) == label_ids).float().mean().item()
        return StepResult(loss=loss, metrics={ACCURACY_METRIC: accuracy})

    def resume_state(self) -> dict:
        return {LABELS_KEY: list(self.labels)}

    def restore_state(self, state: dict) -> None:
        saved_labels = state.get(LABELS_KEY)
        if saved_labels != self.labels:
            message = f"The checkpoint was trained on the labels {saved_labels}; this dataset has {self.labels}."
            raise TrainError(RRS_T2005, message)


def check_rows(rows: list[dict]) -> None:
    """Every row needs a text and a label (RRS-T0003 names the first that doesn't)."""
    for row_index, row in enumerate(rows):
        if not isinstance(row.get(TEXT_COLUMN), str) or not isinstance(row.get(LABEL_COLUMN), str):
            message = f"Row {row_index} has no `{TEXT_COLUMN}` and `{LABEL_COLUMN}` strings for classification."
            raise TrainError(RRS_T0003, message)


def row_texts(rows: list[dict]) -> list[str]:
    return [row[TEXT_COLUMN] for row in rows]


def row_labels(rows: list[dict]) -> list[str]:
    return [row[LABEL_COLUMN] for row in rows]


def vocabulary_size(spec: ModelSpec) -> int:
    """The vocab_size of the embedding node the design's input feeds."""
    node = find_node(spec, spec.inputs[0].encoder_node, EMBEDDING_OP)
    return embedding_attributes_from_json(node.attributes).vocab_size


def class_count(spec: ModelSpec) -> int:
    """The class_count of the classifier head the design's first output is."""
    node = find_node(spec, spec.outputs[0].graph_node, CLASSIFIER_HEAD_OP)
    return classifier_head_attributes_from_json(node.attributes).class_count


def find_node(spec: ModelSpec, node_id: str, op: str) -> GraphNode:
    for node in spec.graph.nodes:
        if node.node_id == node_id and node.op == op:
            return node
    raise TrainError(RRS_T0005, f"Classification needs an {op} node as {node_id}; the design has none.")


def check_label_count(labels: list[str], head_class_count: int) -> None:
    if len(labels) > head_class_count:
        message = f"The dataset has {len(labels)} labels, but the design's classifier head has {head_class_count}."
        raise TrainError(RRS_T0005, message)


def encode_batch(vocabulary: WordVocabulary, rows: list[dict], sequence_length: int) -> torch.Tensor:
    """[batch, sequence_length] word ids: each text's words, cut to the length, then padded with PADDING_ID."""
    token_ids = torch.full((len(rows), sequence_length), PADDING_ID, dtype=torch.long)
    for row_index, row in enumerate(rows):
        word_ids = encode_text(vocabulary, row[TEXT_COLUMN])[:sequence_length]
        token_ids[row_index, : len(word_ids)] = torch.tensor(word_ids, dtype=torch.long)
    return token_ids
