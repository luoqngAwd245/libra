// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module defines traits and implementations of canonical serialization mechanism.
//!
//! A struct can implement the CanonicalSerialize trait to specify how to serialize itself,
//! and the CanonicalDeserialize trait to specify deserialization, if it needs to. One design
//! goal of this serialization format is to optimize for simplicity. It is not designed to be
//! another full-fledged network serialization as Protobuf or Thrift. It is designed
//! for doing only one thing right, which is to deterministically generate consistent bytes
//! from a data structure.
//!
//! A good example of how to use this framework is described in
//! ./canonical_serialization_test.rs
//!
//! An extremely simple implementation of CanonicalSerializer is also provided, the encoding
//! rules are:
//! (All unsigned integers are encoded in little-endian representation unless specified otherwise)
//!
//! 1. The encoding of an unsigned 64-bit integer is defined as its little endian representation
//!    in 8 bytes
//!
//! 2. The encoding of an item (byte array) is defined as:
//!    [length in bytes, represented as 4-byte integer] || [item in bytes]
//!
//!
//! 3. The encoding of a list of items is defined as: (This is not implemented yet because
//!    there is no known struct that needs it yet, but can be added later easily)
//!    [No. of items in the list, represented as 4-byte integer] || encoding(item_0) || ....
//!
//! 4. The encoding of an ordered map where the keys are ordered by lexicographic order.
//!    Currently, we only support key and value of type Vec<u8>. The encoding is defined as:
//!    [No. of key value pairs in the map, represented as 4-byte integer] || encode(key1) ||
//!    encode(value1) || encode(key2) || encode(value2)...
//!    where the pairs are appended following the lexicographic order of the key
//!
//! What is canonical serialization?
//!
//! Canonical serialization guarantees byte consistency when serializing an in-memory
//! data structure. It is useful for situations where two parties want to efficiently compare
//! data structures they independently maintain. It happens in consensus where
//! independent validators need to agree on the state they independently compute. A cryptographic
//! hash of the serialized data structure is what ultimately gets compared. In order for
//! this to work, the serialization of the same data structures must be identical when computed
//! by independent validators potentially running different implementations
//! of the same spec in different languages.
//!
//!该模块定义规范序列化机制的特征和实现。
//!如果需要，结构可以实现CanonicalSerialize特性来指定如何序列化自身，并可以实现CanonicalDeserialize特性来指定反序列化。这种序列化格式的一个设计目标是为简单起见进行优化。它不旨在作为Protobuf或Thrift的另一种成熟的网络序列化。它被设计为仅做一件正确的事，即确定性地从数据结构中生成一致的字节。
//!
//!有关如何使用此框架的一个很好的示例，请参见./canonical_serialization_test.rs
//!
//!还提供了CanonicalSerializer的极其简单的实现，其编码规则为：
//!(除非另有说明，否则所有无符号整数都以little-endian表示形式编码)
//!
//!1.无符号64位整数的编码定义为8字节的小字节序表示
//!
//!2.项（字节数组）的编码定义为：
//![长度（以字节为单位，以4字节整数表示）] || [以字节为单位的项目]
//!
//!
//!3.项目列表的编码定义为：(尚未实现，因为尚无已知结构需要，但稍后可以轻松添加)
//![没有。列表中的项数，表示为4字节整数] ||编码（item_0）|| ....
//!
//!4.有序图的编码，其中键按字典顺序排序。
//!当前，我们仅支持Vec <u8>类型的键和值。编码定义为：
//![没有。映射中的键值对的数量，表示为4字节整数encode（key1）||编码（值1）|| encode（key2）||编码（值2）...
//!这些对按照密钥的字典顺序附加
//!
//!什么是规范序列化？
//!
//!规范化序列化可在序列化内存数据结构时保证字节一致性。这对于两方要有效比较他们独立维护的数据结构的情况
//! 很有用。它发生在共识中，即独立验证者需要就他们独立计算的状态达成一致。最终比较的是序列化数据结构的
//! 加密哈希。为了使它起作用，当由可能在不同语言中运行相同规范的不同实现的独立验证器计算时，相同数据结构
//! 的序列化必须相同

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::prelude::*;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    mem::size_of,
};

pub mod test_helper;

#[cfg(test)]
mod canonical_serialization_test;

// use the signed 32-bit integer's max value as the maximum array length instead of
// unsigned 32-bit integer. This gives us the opportunity to use the additional sign bit
// to signal a length extension to support arrays longer than 2^31 in the future
// 使用有符号的32位整数的最大值作为最大数组长度，而不是无符号的32位整数。 这使我们有机会使用附加符号
// 位来表示长度扩展，以支持将来大于2 ^ 31的数组
const ARRAY_MAX_LENGTH: usize = i32::max_value() as usize;

pub trait CanonicalSerialize {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()>;
}

pub trait CanonicalSerializer {
    fn encode_struct(&mut self, structure: &impl CanonicalSerialize) -> Result<&mut Self>
    where
        Self: std::marker::Sized,
    {
        structure.serialize(self)?;
        Ok(self)
    }

    fn encode_u64(&mut self, v: u64) -> Result<&mut Self>;

    fn encode_u32(&mut self, v: u32) -> Result<&mut Self>;

    fn encode_u16(&mut self, v: u16) -> Result<&mut Self>;

    fn encode_u8(&mut self, v: u8) -> Result<&mut Self>;

    fn encode_bool(&mut self, b: bool) -> Result<&mut Self>;

    // Use this encoder when the length of the array is known to be fixed and always known at
    // deserialization time. The raw bytes of the array without length prefix are encoded.
    // For deserialization, use decode_bytes_with_len() which requires giving the length
    // as input
    // 当数组的长度是固定的并且在反序列化时始终是已知的时，请使用此编码器。 没有长度前缀的数组的原始
    // 字节被编码。 对于反序列化，请使用decode_bytes_with_len（），这需要提供长度作为输入
    fn encode_raw_bytes(&mut self, bytes: &[u8]) -> Result<&mut Self>;

    // Use this encoder to encode variable length byte arrays whose length may not be known at
    // deserialization time.
    // 使用此编码器编码可变长度字节数组，其长度在反序列化时可能未知。
    fn encode_variable_length_bytes(&mut self, v: &[u8]) -> Result<&mut Self>;

    fn encode_btreemap<K: CanonicalSerialize, V: CanonicalSerialize>(
        &mut self,
        v: &BTreeMap<K, V>,
    ) -> Result<&mut Self>;

    fn encode_vec<T: CanonicalSerialize>(&mut self, v: &[T]) -> Result<&mut Self>;
}

type Endianness = LittleEndian;

/// An implementation of a simple canonical serialization format that implements the
/// CanonicalSerializer trait using a byte vector.
/// 一种简单的规范化序列化格式的实现，该格式使用字节向量实现CanonicalSerializer特性。
#[derive(Clone)]
pub struct SimpleSerializer<W> {
    output: W,
}

impl<W> Default for SimpleSerializer<W>
where
    W: Default + std::io::Write,
{
    fn default() -> Self {
        SimpleSerializer::new()
    }
}

impl<W> SimpleSerializer<W>
where
    W: Default + std::io::Write,
{
    pub fn new() -> Self {
        SimpleSerializer {
            output: W::default(),
        }
    }

    /// Create a SimpleSerializer on the fly and serialize `object`
    pub fn serialize(object: &impl CanonicalSerialize) -> Result<W> {
        let mut serializer = Self::default();
        object.serialize(&mut serializer)?;
        Ok(serializer.get_output())
    }

    /// Consume the SimpleSerializer and return the output
    pub fn get_output(self) -> W {
        self.output
    }
}

impl<W> CanonicalSerializer for SimpleSerializer<W>
where
    W: std::io::Write,
{
    fn encode_u64(&mut self, v: u64) -> Result<&mut Self> {
        self.output.write_u64::<Endianness>(v)?;
        Ok(self)
    }

    fn encode_u32(&mut self, v: u32) -> Result<&mut Self> {
        self.output.write_u32::<Endianness>(v)?;
        Ok(self)
    }

    fn encode_u16(&mut self, v: u16) -> Result<&mut Self> {
        self.output.write_u16::<Endianness>(v)?;
        Ok(self)
    }

    fn encode_u8(&mut self, v: u8) -> Result<&mut Self> {
        self.output.write_u8(v)?;
        Ok(self)
    }

    fn encode_bool(&mut self, b: bool) -> Result<&mut Self> {
        let byte: u8 = if b { 1 } else { 0 };
        self.output.write_u8(byte)?;
        Ok(self)
    }

    fn encode_raw_bytes(&mut self, bytes: &[u8]) -> Result<&mut Self> {
        self.output.write_all(bytes.as_ref())?;
        Ok(self)
    }

    fn encode_variable_length_bytes(&mut self, v: &[u8]) -> Result<&mut Self> {
        ensure!(
            v.len() <= ARRAY_MAX_LENGTH,
            "array length exceeded the maximum length limit. \
             length: {}, Max length limit: {}",
            v.len(),
            ARRAY_MAX_LENGTH,
        );

        // first add the length as a 4-byte integer
        self.output.write_u32::<Endianness>(v.len() as u32)?;
        self.output.write_all(v)?;
        Ok(self)
    }

    fn encode_btreemap<K: CanonicalSerialize, V: CanonicalSerialize>(
        &mut self,
        v: &BTreeMap<K, V>,
    ) -> Result<&mut Self> {
        ensure!(
            v.len() <= ARRAY_MAX_LENGTH,
            "map size exceeded the maximum limit. length: {}, max length limit: {}",
            v.len(),
            ARRAY_MAX_LENGTH,
        );

        // add the number of pairs in the map
        self.output.write_u32::<Endianness>(v.len() as u32)?;

        // Regardless of the order defined for K of the map, write in the order of the lexicographic
        // order of the canonical serialized bytes of K
        let mut map = BTreeMap::new();
        for (key, value) in v {
            map.insert(
                SimpleSerializer::<Vec<u8>>::serialize(key)?,
                SimpleSerializer::<Vec<u8>>::serialize(value)?,
            );
        }

        for (key, value) in map {
            self.encode_raw_bytes(&key)?;
            self.encode_raw_bytes(&value)?;
        }
        Ok(self)
    }

    fn encode_vec<T: CanonicalSerialize>(&mut self, v: &[T]) -> Result<&mut Self> {
        ensure!(
            v.len() <= ARRAY_MAX_LENGTH,
            "map size exceeded the maximum limit. length: {}, max length limit: {}",
            v.len(),
            ARRAY_MAX_LENGTH,
        );

        // add the number of items in the vec
        self.output.write_u32::<Endianness>(v.len() as u32)?;
        for value in v {
            self.encode_struct(value)?;
        }
        Ok(self)
    }
}

pub trait CanonicalDeserializer {
    fn decode_struct<T>(&mut self) -> Result<T>
    where
        T: CanonicalDeserialize,
        Self: Sized,
    {
        T::deserialize(self)
    }

    fn decode_u64(&mut self) -> Result<u64>;

    fn decode_u32(&mut self) -> Result<u32>;

    fn decode_u16(&mut self) -> Result<u16>;

    fn decode_u8(&mut self) -> Result<u8>;

    fn decode_bool(&mut self) -> Result<bool>;

    // decode a byte array with the given length as input
    fn decode_bytes_with_len(&mut self, len: u32) -> Result<Vec<u8>>;

    fn decode_variable_length_bytes(&mut self) -> Result<Vec<u8>>;

    fn decode_btreemap<K: CanonicalDeserialize + std::cmp::Ord, V: CanonicalDeserialize>(
        &mut self,
    ) -> Result<BTreeMap<K, V>>;

    fn decode_vec<T: CanonicalDeserialize>(&mut self) -> Result<Vec<T>>;
}

pub trait CanonicalDeserialize {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized;
}

#[derive(Clone)]
pub struct SimpleDeserializer<'a> {
    raw_bytes: Cursor<&'a [u8]>,
}

impl<'a> SimpleDeserializer<'a> {
    pub fn new<T>(raw_bytes: &'a T) -> Self
    where
        T: AsRef<[u8]> + ?Sized,
    {
        Self {
            raw_bytes: Cursor::new(raw_bytes.as_ref()),
        }
    }

    pub fn deserialize<T>(data: &'a [u8]) -> Result<T>
    where
        T: CanonicalDeserialize,
    {
        let mut deserializer = Self::new(data);
        T::deserialize(&mut deserializer)
    }
}

impl<'a> CanonicalDeserializer for SimpleDeserializer<'a> {
    fn decode_u64(&mut self) -> Result<u64> {
        let num = self.raw_bytes.read_u64::<Endianness>()?;
        Ok(num)
    }

    fn decode_u32(&mut self) -> Result<u32> {
        let num = self.raw_bytes.read_u32::<Endianness>()?;
        Ok(num)
    }

    fn decode_u16(&mut self) -> Result<u16> {
        let num = self.raw_bytes.read_u16::<Endianness>()?;
        Ok(num)
    }

    fn decode_u8(&mut self) -> Result<u8> {
        let num = self.raw_bytes.read_u8()?;
        Ok(num)
    }

    fn decode_bool(&mut self) -> Result<bool> {
        let b = self.raw_bytes.read_u8()?;
        ensure!(b == 0 || b == 1, "bool must be 0 or 1, found {}", b,);
        Ok(b != 0)
    }

    fn decode_bytes_with_len(&mut self, len: u32) -> Result<Vec<u8>> {
        // make sure there is enough bytes left in the buffer
        let remain = self.raw_bytes.get_ref().len() as u64 - self.raw_bytes.position();
        ensure!(
            remain >= len.into(),
            "not enough bytes left. input size: {}, remaining: {}",
            len,
            remain
        );

        let mut buffer = vec![0; len as usize];
        self.raw_bytes.read_exact(&mut buffer)?;
        Ok(buffer)
    }

    fn decode_variable_length_bytes(&mut self) -> Result<Vec<u8>> {
        let len = self.raw_bytes.read_u32::<Endianness>()?;
        ensure!(
            len as usize <= ARRAY_MAX_LENGTH,
            "array length longer than max allowed length. len: {}, max: {}",
            len,
            ARRAY_MAX_LENGTH
        );

        // make sure there is enough bytes left in the buffer
        let remain = self.raw_bytes.get_ref().len() - self.raw_bytes.position() as usize;
        ensure!(
            remain >= (len as usize),
            "not enough bytes left. len: {}, remaining: {}",
            len,
            remain
        );

        let mut vec = vec![0; len as usize];
        self.raw_bytes.read_exact(&mut vec)?;
        Ok(vec)
    }

    fn decode_btreemap<K: CanonicalDeserialize + std::cmp::Ord, V: CanonicalDeserialize>(
        &mut self,
    ) -> Result<BTreeMap<K, V>> {
        let len = self.raw_bytes.read_u32::<Endianness>()?;
        ensure!(
            len as usize <= ARRAY_MAX_LENGTH,
            "map size bigger than max allowed. size: {}, max: {}",
            len,
            ARRAY_MAX_LENGTH
        );

        let mut map = BTreeMap::new();
        for _i in 0..len {
            let key = K::deserialize(self)?;
            let value = V::deserialize(self)?;
            map.insert(key, value);
        }
        Ok(map)
    }

    fn decode_vec<T: CanonicalDeserialize>(&mut self) -> Result<Vec<T>> {
        let len = self.raw_bytes.read_u32::<Endianness>()?;
        ensure!(
            len as usize <= ARRAY_MAX_LENGTH,
            "map size bigger than max allowed. size: {}, max: {}",
            len,
            ARRAY_MAX_LENGTH
        );

        let mut vec = Vec::new();
        for _i in 0..len {
            let v = T::deserialize(self)?;
            vec.push(v);
        }
        Ok(vec)
    }
}

impl<T> CanonicalSerialize for Vec<T>
where
    T: CanonicalSerialize,
{
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_vec(self.as_ref())?;
        Ok(())
    }
}

impl<T> CanonicalDeserialize for Vec<T>
where
    T: CanonicalDeserialize,
{
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        deserializer.decode_vec()
    }
}

impl CanonicalSerialize for u16 {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_u16(*self)?;
        Ok(())
    }
}

impl CanonicalDeserialize for u16 {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self> {
        deserializer.decode_u16()
    }
}

impl CanonicalSerialize for u32 {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_u32(*self)?;
        Ok(())
    }
}

impl CanonicalDeserialize for u32 {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        deserializer.decode_u32()
    }
}

impl CanonicalSerialize for i32 {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_u32(*self as u32)?;
        Ok(())
    }
}

impl CanonicalDeserialize for i32 {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        let num = deserializer.decode_u32()? as i32;
        Ok(num)
    }
}

impl CanonicalSerialize for u64 {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_u64(*self)?;
        Ok(())
    }
}

impl CanonicalDeserialize for u64 {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        let num = deserializer.decode_u64()?;
        Ok(num)
    }
}

impl CanonicalSerialize for i64 {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_u64(*self as u64)?;
        Ok(())
    }
}

impl CanonicalDeserialize for i64 {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        let num = deserializer.decode_u64()? as i64;
        Ok(num)
    }
}

impl CanonicalSerialize for usize {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        assert_eq!(8, size_of::<usize>());
        serializer.encode_u64(*self as u64)?;
        Ok(())
    }
}

impl CanonicalDeserialize for usize {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        assert_eq!(8, size_of::<usize>());
        let num = deserializer.decode_u64()? as usize;
        Ok(num)
    }
}

impl CanonicalSerialize for u8 {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_u8(*self)?;
        Ok(())
    }
}

impl CanonicalDeserialize for u8 {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        let num = deserializer.decode_u8()?;
        Ok(num)
    }
}

impl CanonicalSerialize for BTreeMap<Vec<u8>, Vec<u8>> {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_btreemap(self)?;
        Ok(())
    }
}

impl CanonicalDeserialize for BTreeMap<Vec<u8>, Vec<u8>> {
    fn deserialize(deserializer: &mut impl CanonicalDeserializer) -> Result<Self>
    where
        Self: Sized,
    {
        Ok(deserializer.decode_btreemap()?)
    }
}
