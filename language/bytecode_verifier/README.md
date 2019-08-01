---
id: bytecode-verifier 
title: Bytecode Verifier 字节验证程序
custom_edit_url: https://github.com/libra/libra/edit/master/language/bytecode_verifier/README.md
---

# Bytecode Verifier
字节验证程序：用于检查堆栈使用、类型、资源、引用的安全

## Overview 概要

The bytecode verifier contains a static analysis tool for rejecting invalid Move bytecode. It checks the safety of stack usage, types, resources, and references.

The body of each function in a compiled module is verified separately while trusting the correctness of function signatures in the module. Checking that each function signature matches its definition is a separate responsibility. The body of a function is a sequence of bytecode instructions. This instruction sequence is checked in several phases described below.

字节码验证器包含一个静态分析工具，用于拒绝无效的Move字节码。 它检查堆栈使用，类型，资源和引用的安全性。

编译模块中每个函数的主体单独验证，同时信任模块中函数签名的正确性。 检查每个函数签名是否与其定义匹配是一项单独的责任。 函数体是一系列字节码指令。 在下面描述的几个阶段中检查该指令序列。

## CFG Construction  CFG构造

A control-flow graph is constructed by decomposing the instruction sequence into a collection of basic blocks. Each basic block contains a contiguous sequence of instructions; the set of all instructions is partitioned among the blocks. Each block ends with a branch or return instruction. The decomposition into blocks guarantees that branch targets land only at the beginning of some block. The decomposition also attempts to ensure that the generated blocks are maximal. However, the soundness of the analysis does not depend on maximality.

通过将指令序列分解为基本块的集合来构造控制流图。每个基本快包含一些列指令序列；包含所有指令的集合在块之间分区。每个块包含一个分支或返回指令。
分解指令到块保证分支目标仅在某个区块的开始处。分解同时也试图确认生成的块数目最大。然而，分析的可靠性不依赖于最大化。


## Stack Safety 堆栈安全

The execution of a block happens in the context of a stack and an array of local variables. The parameters of the function are a prefix of the array of local variables. Arguments and return values are passed across function calls via the stack. When a function starts executing, its arguments are already loaded into its parameters. Suppose the stack height is *n* when a function starts executing; then valid bytecode must enforce the invariant that when execution lands at the beginning of a basic block, the stack height is *n*. Furthermore, at a return instruction, the stack height must be *n*+*k* where *k*, s.t. *k*>=0 is the number of return values. The first phase of the analysis checks that this invariant is maintained by analyzing each block separately, calculating the effect of each instruction in the block on the stack height, checking that the height does not go below *n*, and that is left either at *n* or *n*+*k* (depending on the final instruction of the block and the return type of the function) at the end of the block.

block在一个包含本地变量数组和一个堆栈的上下文中执行。函数的实参是局部变量数组的前缀。在函数调用之间传递参数和返回值是通过堆栈完成的。
当函数开始执行时，其实参通过堆栈已经加载到其参数中。假设当寒素开始执行时，堆栈的高度是'n'。而且，在一个返回指令中，堆栈的高度必须
是 n+ k ,k满足 k>= 0,k是返回值的个数。分析的第一个阶段通过单独分析每个块检查不变量的维持，计算块中每条指令对堆栈高度的影响，检查
检查高度不低于* n *，在块的尾部留下n 或者 n+k（取决于块中的最后的指令和函数的返回类型）

## Type Safety 类型安全

The second phase of the analysis checks that each operation, primitive or defined function, is invoked with arguments of appropriate types. The operands of an operation are values located either in a local variable or on the stack. The types of local variables of a function are already provided in the bytecode. However, the types of stack values are inferred. This inference and the type checking of each operation can be done separately for each block. Since the stack height at the beginning of each block is *n* and does not go below *n* during the execution of the block, we only need to model the suffix of the stack starting at *n* for type checking the block instructions. We model this suffix using a stack of types on which types are pushed and popped as the instruction stream in a block is processed. Only the type stack and the statically-known types of local variables are needed to type check each instruction.

分析的第二阶段检查是否使用适当类型的参数调用每个操作，原始函数或已定义函数。操作的操作数是位于局部变量或堆栈中的值。本地变量的类型已经
在字节码中被提供。然而，堆栈值的类型是通过推断出来的。每个块每个操作的类型推断和检查分开做。既然堆栈的高度在每个块的开始定为n并且在执行
过程中不低于n，我们只需要从* n *开始对堆栈的后缀进行建模，以便对块指令进行类型检查。我们建模这个后缀使用一堆类型，当处理块中的指令流时，
这些类型被推送和弹出。这个类型堆栈和本地静态已知类型足够应付每一条指令的类型检查。

## Resource Safety 资源安全

Resources represent the assets of the blockchain. As such, there are certain restrictions on these types that do not apply to normal values. Intuitively, resource values cannot be copied and must be used by the end of the transaction (this means that they are moved to global storage or destroyed). Concretely, the following restrictions apply:
资源代表区块链的资产。 因此，对这些类型存在某些限制，这些限制不适用于正常值。直观地说，资源值无法复制，必须在事务结束时使用（这意味着
 移动到全局存储或销毁）。具体而言，有以下一些限制：
* `CopyLoc` and `StLoc` require that the type of local is not of resource kind.
* `WriteRef`, `Eq`, and `Neq` require that the type of the reference is not of resource kind.
* At the end of a function (when `Ret` is reached), no local whose type is of resource kind must be empty, i.e., the value must have been moved out of the local.
* `CopyLoc` and `StLoc` 要求本地类型不能是资源中的一种
* `WriteRef`, `Eq`, and `Neq`要求引用类型不能是资源中的一种
* 在函数结束处(当 `Ret`到达的位置),任何本地的资源都不能为空，i.e, 值必须被move出本地。

As mentioned above, this last rule around `Ret` implies that the resource *must* have been either:
正如上面提及的，最后一条关于’Ret‘的规则意味着资源必须满足以下两条之一：

* Moved to global storage via `MoveToSender`.
* Destroyed via `Unpack`.
* Moved 到全局存储 通过`MoveToSender`
* 销毁 通过 `Unpack`

Both `MoveToSender` and `Unpack` are internal to the module in which the resource is declared.
“MoveToSender”和“Unpack”都是声明资源的模块的内部。
## Reference Safety 引用安全

References are first-class in the bytecode language. Fresh references become available to a function in several ways:
引用在字节码语言中是第一等的。函数可以通过以下几种方式获得新的引用:
* Inputing parameters.
* Taking the address of the value in a local variable.
* Taking the address of the globally published value in an address.
* Taking the address of a field from a reference to the containing struct.
* Returning value from a function.
* 输入参数
* 获取局部变量的值的地址
* 获取全局发布地址值
* 从对包含字段结构的引用中获取字段的地址
* 函数返回值

The goal of reference safety checking is to ensure that there are no dangling references. Here are some examples of dangling references:
引用的安全检查的目标是确保没有悬垂引用，下面就是几个悬垂引用的例子：
* Local variable `y` contains a reference to the value in a local variable `x`; `x` is then moved.
* Local variable `y` contains a reference to the value in a local variable `x`; `x` is then bound to a new value.
* Reference is taken to a local variable that has not been initialized.
* Reference to a value in a local variable is returned from a function.
* Reference `r` is taken to a globally published value `v`; `v` is then unpublished.
* 局部变量y包含一个指向本地变量x的值的引用；x被move
* 局部变量y包含一个指向本地变量x的值的引用；绑定一个新值
* 引用指向一个未初始化的局部变量
* 引用指向一个局部变量，局部变量被函数返回
* 引用 r 指向全局发布的值 v; v 则未发布。

References can be either exclusive or shared; the latter allow read-only access. A secondary goal of reference safety checking is to ensure that in the execution context of the bytecode program — including the entire evaluation stack and all function frames — if there are two distinct storage locations containing references `r1` and `r2` such that `r2` extends `r1`, then both of the following conditions hold:
引用既可以独享也可以共享；后者只允许只读。第二个次要目标安全检查是确保在字节码程序的执行上下文中 - 包括整个
评估堆栈和所有函数帧 - 如果有两个不同的存储位置包含引用`r1`和`r2`使`r2`扩展`r1`，使得以下两个条件都成立

* If `r1` is tagged as exclusive, then it must be inactive, i.e. it is impossible to reach a control location where `r1` is dereferenced or mutated.
* If `r1` is shared, then `r2` is shared.
* 如果r1被标记独占， 则它必须是不活动的，即 r1 不能被解除引用或修改。
* 如果 r1 是共享的, 那么 r2 也是共享的。
The two conditions above establish the property of referential transparency, important for scalable program verification, which looks roughly as follows: consider the piece of code `v1 = *r; S; v2 = *r`, where `S` is an arbitrary computation that does not perform any write through the syntactic reference `r` (and no writes to any `r'` that extends `r`). Then `v1 == v2`.
上述两个条件确立了引用透明的特性，这对于可扩展程序验证很重要，其大致如下：考虑一段代码v1 = * r; S; v2 = * r，
其中S是一个任意计算，它不通过语法引用 r 执行任何写操作（并且没有写任何扩展 r 的 r' ）。 然后 v1 == v2 。
### Analysis Setup 分析设置

The reference safety analysis is set up as a flow analysis (or abstract interpretation). An abstract state is defined for abstractly executing the code of a basic block. A map is maintained from basic blocks to abstract states. Given an abstract state *S* at the beginning of a basic block *B*, the abstract execution of *B* results in state *S'*. This state *S'* is propagated to all successors of *B* and recorded in the map. If a state already existed for a block, the freshly propagated state is “joined” with the existing state. If the join fails an error is reported. If the join succeeds but the abstract state remains unchanged, no further propagation is done. Otherwise, the state is updated and propagated again through the block. An error may also be reported when an instruction is processed during the propagation of abstract state through a block.
引用安全分析设置为流分析（或者对熟悉概念的那些人的抽象解释）。抽象状态定义为抽象执行基本块代码。从基本块到抽象状态
 的map被维持。假设 在基本块B的开头一个抽象类型S，B的抽象执行结果在状态S’中。该状态* S'*传播到* B *的所有后继者并记录在map中。
 如果一个状态已经存在一个块中， 新的状态 和已经存在的状态 'Joined'。Join可能会失败，在这种情况下会报告错误。如果join成功但
 抽象状态保持不变，则不再进行传播。否则，状态更新并再次通过块传播。在抽象状态通过块传播期间处理指令时，也可能报告错误。传播中断因为
 
### Abstract State 抽象状态

The abstract state has three components:
抽象状态有三个组成部分：
* A partial map from locals to abstract values. Locals that are not in the domain of this map are unavailable. Availability is a generalization of the concept of being initialized. A local variable may become unavailable subsequent to initialization as a result of being moved. An abstract value is either *Reference*(*n*) (for variables of reference type) or *Value*(*ns*) (for variables of value type), where *n* is a nonce and *ns* is a set of nonces. A nonce is a constant used to represent a reference. Let *Nonce* represent the set of all nonces. If a local variable *l* is mapped to *Value*(*ns*), it means that there are outstanding borrowed references pointing into the value stored in *l*. For each member *n* of *ns*, there must be a local variable *l* mapped to *Reference*(*n*). If a local variable *x* is mapped to *Reference*(*n*) and there are local variables *y* and *z* mapped to *Value*(*ns1*) and *Value*(*ns2*) respectively, then it is possible that *n* is a member of both *ns1* and *ns2*. This simply means that the analysis is lossy. The special case when *l* is mapped to *Value*({}) means that there are no borrowed references to *l*, and, therefore, *l* may be destroyed or moved.
* 从局部到抽象值的部分映射。不在此映射范围内的本地不可用。 可用是初始化概念的一个概括。由于被移动，局部变量在初始化之后可能变得不可用。抽象值是
 Reference(n)（对于引用类型的变量）或 Value(ns)（对于值类型的变量），其中n是nonce， ns是一组nonces。A nonce 一个常量被用来代表一个引用。让Nonce
 代表所有nonce的集合。如果一个局部变量映射到value（ns）， 意味着明显的借用引用指向存储在l中的值。对ns的每一个成员 n来说，必须有一个局部变量l
 映射到Reference（n).如果一个局部变量 x被映射到Reference（n),有局部变量 y和z分别映射到Value（ns1) 和value（ns2), n可能是ns1 和ns2 的一个成员。
 这简单意味者分析是有损的。* l *映射到* Value *（{}）时的特殊情况意味着没有对* l *的借用引用，因此* l *可能被销毁或移动。

* The partial map from locals to abstract values is not enough by itself to check bytecode programs because values manipulated by the bytecode can be large nested structures with references pointing into the middle. A reference pointing into the middle of a value could be extended to produce another reference. Some extensions should be allowed but others should not. To keep track of relative extensions among references, the abstract state has a second component. This component is a map from nonces to one of the following two kinds of borrowed information:
* A set of nonces.
* A map from fields to sets of nonces.

The current implementation stores this information as two separate maps with disjointed domains:
  * *borrowed_by* maps from *Nonce* to *Set*<*Nonce*>.
  * *fields_borrowed_by* maps from *Nonce* to *Map*<*Field*, *Set*<*Nonce*>>.
      * If *n2* in *borrowed_by*[*n1*], then it means that the reference represented by *n2* is an extension of the reference represented by *n1*.
      * If *n2* in *fields_borrowed_by*[*n1*][*f*], it means that the reference represented by *n2* is an extension of the *f*-extension of the reference represented by *n1*. Based on this intuition, it is a sound overapproximation to move a nonce *n* from the domain of *fields_borrowed_by* to the domain of *borrowed_by* by taking the union of all nonce sets corresponding to all fields in the domain of *fields_borrowed_by*[*n*].
* To propagate an abstract state across the instructions in a block, the values and references on the stack must also be modeled. We had earlier described how we model the usable stack suffix as a stack of types. We now augment the contents of this stack to be a structure containing a type and an abstract value. We maintain the invariant that non-reference values on the stack cannot have pending borrows on them. Therefore, if there is an abstract value *Value*(*ns*) on the stack, then *ns* is empty.

### Values and References

Let us take a closer look at how values and references, shared and exclusive, are modeled.

* A non-reference value is modeled as *Value*(*ns*) where *ns* is a set of nonces representing borrowed references. Destruction/move/copy of this value is deemed safe only if *ns* is empty. Values on the stack trivially satisfy this property, but values in local variables may not.
* A reference is modeled as *Reference*(*n*), where *n* is a nonce. If the reference is tagged as shared, then read access is always allowed and write access is never allowed. If a reference *Reference*(*n*) is tagged exclusive, write access is allowed only if *n* does not have a borrow, and read access is allowed if all nonces that borrow from *n* reside in references that are tagged as shared. Furthermore, the rules for constructing references guarantee that an extension of a reference tagged as shared must also be tagged as shared. Together, these checks provide the property of referential transparency mentioned earlier.

At the moment, the bytecode language does not contain any direct constructors for shared references. `BorrowLoc` and `BorrowGlobal` create exclusive references. `BorrowField` creates a reference that inherits its tag from the source reference. Move (when applied to a local variable containing a reference) moves the reference from a local variable to the stack. `FreezeRef` is used to convert an existing exclusive reference to a shared reference. In the future, we may add a version of `BorrowGlobal` that generates a shared reference

**Errors.** As mentioned earlier, an error is reported by the checker in one of the following situations:

* An instruction cannot be proven to be safe during the propagation of the abstract state through a block.
* Join of abstract states propagated via different incoming edges into a block fails.

Let us take a closer look at the second reason for error reporting above. Note that the stack of type and abstract value pairs representing the usable stack suffix is empty at the beginning of a block. So, the join occurs only over the abstract state representing the available local variables and the borrow information. The join fails only in the situation when the set of available local variables is different on the two edges. If the set of available variables is identical, the join itself is straightforward &mdash; the borrow sets are joined point-wise. There are two subtleties worth mentioning though:

* The set of nonces used in the abstract states along the two edges may not have any connection to each other. Since the actual nonce values are immaterial, the nonces are canonically mapped to fixed integers (indices of local variables containing the nonces) before performing the join.
* During the join, if a nonce *n* is in the domain of borrowed_by on one side and in the domain of fields_borrowed_by on the other side, *n* is moved from fields_borrowed_by to borrowed_by before doing the join.

### Borrowing References

Each of the reference constructors ---`BorrowLoc`, `BorrowField`, `BorrowGlobal`, `FreezeRef`, and `CopyLoc`--- is modeled via the generation of a fresh nonce. While `BorrowLoc` borrows from a value in a local variable, `BorrowGlobal` borrows from the global pool of values. `BorrowField`, `FreezeRef`, and `CopyLoc` (when applied to a local containing a reference) borrow from the source reference. Since each fresh nonce is distinct from all previously-generated nonces, the analysis maintains the invariant that all available local variables and stack locations of reference type have distinct nonces representing their abstract value. Another important invariant is that every nonce referred to in the borrow information must reside in some abstract value representing a local variable or a stack location.

### Releasing References.

References, both global and local, are released by the `ReleaseRef` operation. It is an error to return from a function with unreleased references in a local variable of the function. All references must be explicitly released. Therefore, it is an error to overwrite an available reference using the `StLoc` operation.

References are implicitly released when consumed by the operations `ReadRef`, `WriteRef`, `Eq`, `Neq`, and `EmitEvent`.

### Global References

The safety of global references depends on a combination of static and dynamic analysis. The static analysis does not distinguish between global and local references. But the dynamic analysis distinguishes between them and performs reference counting on the global references as follows: the bytecode interpreter maintains a map `M` from an address and fully-qualified resource type pair to a union (Rust enum) comprising the following values:

* `Empty`
* `RefCount(n)` for some `n` >= 0

Extra state updates and checks are performed by the interpreter for the following operations. In the code below, assert failure indicates a programmer error, and panic failure indicates internal error in the interpreter.

```text
MoveFrom<T>(addr) {
    assert M[addr, T] == RefCount(0);
    M[addr, T] := Empty;
}

MoveToSender<T>(addr) {
    assert M[addr, T] == Empty;
    M[addr, T] := RefCount(0);
}

BorrowGlobal<T>(addr) {
    if let RefCount(n) = M[addr, T] then {
        assert n == 0;
        M[addr, T] := RefCount(n+1);
    } else {
        assert false;
    }
}

CopyLoc(ref) {
    if let Global(addr, T) = ref {
        if let RefCount(n) = M[addr, T] then {
            assert n > 0;
            M[addr, T] := RefCount(n+1);
        } else {
            panic false;
        }
    }
}

ReleaseRef(ref) {
    if let Global(addr, T) = ref {
        if let RefCount(n) = M[addr, T] then {
            assert n > 0;
            M[addr, T] := RefCount(n-1);
        } else {
            panic false;
        }
    }
}
```

A subtle point not explicated by the rules above is that `BorrowField` and `FreezeRef`, when applied to a global reference, leave the reference count unchanged. This is because these instructions consume the reference at the top of the stack while producing an extension of it at the top of the stack. Similarly, since `ReadRef`, `WriteRef`, `Eq`, `Neq`, and `EmitEvent` consume the reference at the top of the stack, they will reduce the reference count by 1.

## How is this module organized?

```text
*
├── invalid_mutations  # Library used by proptests
├── src                # Core bytecode verifier files
├── tests              # Proptests
```
