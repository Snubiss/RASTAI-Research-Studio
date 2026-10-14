//! The model hash (contracts 8.6) over real safetensors bytes: the planner writes the files, the reader reads them back
//! and `model_hash` names the tensors. The re-shard test is the one the issue asks for: the same tensors in one file
//! and in several shards give one hash.

#![allow(clippy::unwrap_used)]

use std::cell::Cell;

use rastai_formats::byte_source::{ByteSource, SliceSource};
use rastai_formats::error::FormatsError;
use rastai_formats::hashing::{hash_bytes, model_hash};
use rastai_formats::safetensors::dtypes::{SafetensorsDtype, dtype_element_bytes};
use rastai_formats::safetensors::writer::{TensorPlan, plan_safetensors};

/// One tensor as a test writes it: its name, dtype, shape and the bytes its data section holds.
struct Tensor {
    name: &'static str,
    dtype: SafetensorsDtype,
    shape: &'static [u64],
    data: Vec<u8>,
}

/// Four tensors whose data bytes are all different, so a swapped or changed byte can't go unnoticed.
fn model_tensors() -> Vec<Tensor> {
    vec![
        tensor("embed", SafetensorsDtype::F32, &[2], 0),
        tensor("head", SafetensorsDtype::Bf16, &[3], 40),
        tensor("layer.0.weight", SafetensorsDtype::U8, &[4], 80),
        tensor("norm", SafetensorsDtype::I32, &[1], 120),
    ]
}

/// A tensor whose data is its element count times its dtype's size, counting up from `first`, so each tensor's data
/// differs from the others.
fn tensor(name: &'static str, dtype: SafetensorsDtype, shape: &'static [u64], first: u8) -> Tensor {
    let element_count: u64 = shape.iter().product();
    let size = element_count as usize * dtype_element_bytes(dtype) as usize;
    Tensor {
        name,
        dtype,
        shape,
        data: (0..size).map(|index| first.wrapping_add(index as u8)).collect(),
    }
}

/// A whole safetensors file holding `tensors` (in any order) and `metadata`, laid out by the planner.
fn file_bytes(tensors: &[Tensor], metadata: &[(&str, &str)]) -> Vec<u8> {
    let plans: Vec<TensorPlan> = tensors
        .iter()
        .map(|tensor| TensorPlan {
            name: tensor.name.to_string(),
            dtype: tensor.dtype,
            shape: tensor.shape.to_vec(),
        })
        .collect();
    let metadata: Vec<(String, String)> = metadata
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let layout = plan_safetensors(&plans, &metadata).unwrap();
    let mut bytes = (layout.header_bytes.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(&layout.header_bytes);
    for entry in &layout.entries {
        let tensor = tensors.iter().find(|tensor| tensor.name == entry.name).unwrap();
        bytes.extend_from_slice(&tensor.data);
    }
    bytes
}

fn hash_of_files(files: &[Vec<u8>]) -> Result<String, String> {
    let sources: Vec<SliceSource> = files.iter().map(|bytes| SliceSource { bytes }).collect();
    let refs: Vec<&dyn ByteSource> = sources.iter().map(|source| source as &dyn ByteSource).collect();
    model_hash(&refs).map_err(|diagnostic| diagnostic.code)
}

fn hash_of_one(bytes: Vec<u8>) -> String {
    hash_of_files(&[bytes]).unwrap()
}

#[test]
fn the_same_tensors_in_one_file_and_in_shards_give_one_hash() {
    let tensors = model_tensors();
    let whole = hash_of_one(file_bytes(&tensors, &[]));
    let shard_one = file_bytes(&tensors[..2], &[("format", "pt")]);
    let shard_two = file_bytes(&tensors[2..3], &[("step", "30")]);
    let shard_three = file_bytes(&tensors[3..], &[]);
    let sharded = hash_of_files(&[shard_one.clone(), shard_two.clone(), shard_three.clone()]).unwrap();
    assert_eq!(sharded, whole);
    // The order the shards are given in doesn't matter either.
    let reversed = hash_of_files(&[shard_three, shard_two, shard_one]).unwrap();
    assert_eq!(reversed, whole);
}

#[test]
fn metadata_does_not_change_the_hash() {
    let tensors = model_tensors();
    let plain = hash_of_one(file_bytes(&tensors, &[]));
    let with_metadata = hash_of_one(file_bytes(&tensors, &[("format", "pt"), ("note", "anything")]));
    assert_eq!(with_metadata, plain);
}

#[test]
fn a_changed_tensor_byte_changes_the_hash() {
    let tensors = model_tensors();
    let before = hash_of_one(file_bytes(&tensors, &[]));
    let mut changed = model_tensors();
    changed[1].data[2] ^= 1;
    assert_ne!(hash_of_one(file_bytes(&changed, &[])), before);
}

#[test]
fn a_changed_name_dtype_or_shape_changes_the_hash() {
    let before = hash_of_one(file_bytes(&model_tensors(), &[]));
    // Same bytes, one thing different each time.
    let renamed = model_tensors()
        .into_iter()
        .map(|tensor| Tensor {
            name: if tensor.name == "norm" {
                "norm_final"
            } else {
                tensor.name
            },
            ..tensor
        })
        .collect::<Vec<Tensor>>();
    assert_ne!(hash_of_one(file_bytes(&renamed, &[])), before);

    let mut retyped = model_tensors();
    retyped[3].dtype = SafetensorsDtype::F32;
    assert_ne!(hash_of_one(file_bytes(&retyped, &[])), before);

    let mut reshaped = model_tensors();
    reshaped[0].shape = &[1, 2];
    assert_ne!(hash_of_one(file_bytes(&reshaped, &[])), before);
}

#[test]
fn the_listing_is_one_line_per_tensor_in_name_order() {
    // The name `b "c"` has a space and quotes; as a JSON string it can't blur into the next field.
    let quoted = Tensor {
        name: "b \"c\"",
        dtype: SafetensorsDtype::U8,
        shape: &[2],
        data: vec![1, 2],
    };
    let plain = Tensor {
        name: "a",
        dtype: SafetensorsDtype::F32,
        shape: &[],
        data: vec![0, 0, 128, 63],
    };
    // The expected listing is written out by hand, so the test doesn't use the code it checks.
    let first_line = format!("\"a\" F32 [] {}\n", hash_bytes(&plain.data));
    let second_line = format!("\"b \\\"c\\\"\" U8 [2] {}\n", hash_bytes(&quoted.data));
    let expected = hash_bytes(format!("{first_line}{second_line}").as_bytes());
    assert_eq!(hash_of_one(file_bytes(&[quoted, plain], &[])), expected);
}

#[test]
fn two_files_holding_one_tensor_name_are_refused_as_unplannable() {
    let tensors = model_tensors();
    let first = file_bytes(&tensors[..1], &[]);
    let again = file_bytes(&tensors[..1], &[("copy", "2")]);
    assert_eq!(hash_of_files(&[first, again]).unwrap_err(), "RRS-E1007");
}

/// A source that remembers the largest read it was asked for, so a test can see whether a tensor was read whole.
struct LargestReadSource<'a> {
    bytes: &'a [u8],
    largest_read: Cell<usize>,
}

impl ByteSource for LargestReadSource<'_> {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), FormatsError> {
        self.largest_read.set(self.largest_read.get().max(buffer.len()));
        SliceSource { bytes: self.bytes }.read_at(offset, buffer)
    }
}

#[test]
fn a_big_tensor_is_hashed_in_bounded_reads() {
    // Three mebibytes of data: a read of the whole tensor at once would be three times the chunk size.
    const BIG_LENGTH: usize = 3 * 1024 * 1024;
    const LARGEST_CHUNK: usize = 1024 * 1024;
    let data: Vec<u8> = (0..BIG_LENGTH).map(|index| (index % 251) as u8).collect();
    let expected_data_hash = hash_bytes(&data);
    let bytes = file_bytes(
        &[Tensor {
            name: "embed",
            dtype: SafetensorsDtype::U8,
            shape: &[BIG_LENGTH as u64],
            data,
        }],
        &[],
    );
    let source = LargestReadSource {
        bytes: &bytes,
        largest_read: Cell::new(0),
    };
    let files: [&dyn ByteSource; 1] = [&source];
    let hash = model_hash(&files).unwrap();
    assert!(
        source.largest_read.get() <= LARGEST_CHUNK,
        "read {} bytes at once",
        source.largest_read.get()
    );
    let listing = format!("\"embed\" U8 [{BIG_LENGTH}] {expected_data_hash}\n");
    assert_eq!(hash, hash_bytes(listing.as_bytes()));
}
