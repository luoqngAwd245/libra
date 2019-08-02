# Contributing to Libra  为Libra贡献

Our goal is to make contributing to the Libra project easy and transparent.
我们的目标是使为Libra贡献更容易和透明

> **Note**: As the Libra Core project is currently an early-stage prototype, it
> is undergoing rapid development. While we welcome contributions, before
> making substantial contributions be sure to discuss them in the Discourse
> forum to ensure that they fit into the project roadmap.
> **注意**: 由于Libra Core项目目前是一个早期阶段的原型，它正在快速发展。 虽然我们欢迎贡献，
> 但在做出实质性贡献之前，一定要在话语论坛上讨论它们，以确保它们符合项目路线图。

## On Contributing 论贡献

### Libra Core

To contribute to the Libra Core implementation, first start with the proper
development copy.
要为Libra Core实现做出贡献，首先要从正确的开发副本开始。

To get the development installation with all the necessary dependencies for
linting, testing, and building the documentation, run the following:
要使开发安装具有用于linting，测试和构建文档的所有必需依赖项，请运行以下命令：
```bash
git clone https://github.com/libra/libra.git
cd libra
./scripts/dev_setup.sh
cargo build
cargo test
```

## Our Development Process 我们的开发过程

#### Code Style, Hints, and Testing 代码样式，提示和测试

Refer to our [Coding
Guidelines](https://developers.libra.org/docs/community/coding-guidelines) for
detailed guidance about how to contribute to the project.
有关如何为项目做出贡献的详细指导，请参阅我们的[编码指南](https://developers.libra.org/docs/community/coding-guidelines)。

#### Documentation 文档

Libra's website is also open source (the code can be found in this
[repository](https://github.com/libra/website/)).  It is built using
[Docusaurus](https://docusaurus.io/):
Libra的网站也是开源的（代码可以在这里找到[储存库（https://github.com/libra/website/））。 
它是使用[Docusaurus]（https://docusaurus.io/）构建的：

If you know Markdown, you can already contribute! This lives in the [website
repo](https://github.com/libra/website).
如果你了解Markdown，你已经可以贡献了！ 它位于[网站repo]（https://github.com/libra/website）。

## Developer Workflow 开发者工作流

Changes to the project are proposed through pull requests. The general pull
request workflow is as follows:
通过拉取请求提出对项目的更改。 一般拉取请求工作流程如下：

1. Fork the repo and create a topic branch off of `master`. 
   fork the repo并从`master`创建一个主题分支。
2. If you have added code that should be tested, add unit tests.
   如果你添加了代码，这些代码需要被测试过，同时加上测试用例
3. If you have changed APIs, update the documentation. Make sure the
   documentation builds. 
   如果您更改了API，请更新文档。 确保文档构建。   
4. Ensure all tests and lints pass on each and every commit that is part of
   your pull request.
   确保所有测试和lint都传递作为pull请求一部分的每个提交。   
5. If you haven't already, complete the Contributor License Agreement (CLA).
   如果您还没有，请完成“贡献者许可协议”（CLA）。
6. Submit your pull request.
   提交你的拉取请求

## Authoring Clean Commits 编写清洁提交

#### Logically Separate Commits 逻辑上分开的提交

Commits should be
[atomic](https://en.wikipedia.org/wiki/Atomic_commit#Atomic_commit_convention)
and broken down into logically separate changes. Diffs should also be made easy
for reviewers to read and review so formatting fixes or code moves should not
be included in commits with actual code changes.
提交应该是[原子]（https://en.wikipedia.org/wiki/Atomic_commit#Atomic_commit_convention）
并分解为逻辑上分开的变化。 差异也应该变得容易审阅者阅读和审阅，因此格式修复或代码移动不应该
包含在具有实际代码更改的提交中。

#### Meaningful Commit Messages 有意义的提交信息

Commit messages are important and incredibly helpful for others when they dig
through the commit history in order to understand why a particular change
was made and what problem it was intending to solve. For this reason commit
messages should be well written and conform with the following format:

All commit messages should begin with a single short (50 character max) line
summarizing the change and should skip the full stop. This is the title of the
commit. It is also preferred that this summary be prefixed with "[area]" where
the area is an identifier for the general area of the code being modified, e.g.
提交消息对于其他人在挖掘提交历史记录时非常重要并且非常有用，以便了解特定更改的原因以及打算解决的问题。 因此，提交消息应该写得很好，并符合以下格式：

所有提交消息都应以一个简短（最多50个字符）的行开头，该行总结了更改并应跳过句号。 这是提交的标题。 还优选的是，该摘要以“[area]”为前缀，其中该区域是被修改的代码的一般区域的标识符，例如

```
* [ci] enforce whitelist of nightly features 强制执行夜间功能的白名单
* [language] removing VerificationPass trait 删除VerificationPass trait
```


A non-exhaustive list of some other areas include:
其他一些领域的非详尽清单包括：
* consensus 一致性模块
* mempool 内存模块
* network 网络
* storage 存储
* execution 执行模块
* vm 虚拟机

Following the commit title (unless it alone is self-explanatory), there should
be a single blank line followed by the commit body which includes more
detailed, explanatory text as separate paragraph(s). It is recommended that the
commit body be wrapped at 72 characters so that Git has plenty of room to
indent the text while still keeping everything under 80 characters overall.
在提交标题之后（除非它本身是不言自明的），应该有一个空行，后面是提交体，其中包含更详细的解释性
文本作为单独的段落。建议将提交体包装为72个字符，这样Git就有足够的空间来缩进文本，同时仍然保持
整体不超过80个字符。

The commit body should provide a meaningful commit message, which:
提交主体应提供有意义的提交消息，其中：
* Explains the problem the change tries to solve, i.e. what is wrong
  with the current code without the change. 解释变化试图解决的问题，即什么是错误的
   使用当前代码而不进行更改。
* Justifies the way the change solves the problem, i.e. why the
  result with the change is better. 证明改变解决问题的方式，即为什么
   结果随着变化更好。
* Alternative solutions considered but discarded, if any. 如果有的话，考虑但丢弃的替代解决方案

#### References in Commit Messages 在提交信息中的引用

If you want to reference a previous commit in the history of the project, use
the format "abbreviated sha1 (subject, date)", with the subject enclosed in a
pair of double-quotes, like this:
如果要在项目历史记录中引用先前的提交，请使用“缩写为sha1（subject，date）”格式，
主题用双引号括起来，如下所示：

```bash
Commit 895b53510 ("[consensus] remove slice_patterns feature", 2019-07-18)
noticed that ...
```

This invocation of `git show` can be used to obtain this format:
如果这个`git show`的调用可以用来获得这种格式：

```bash
git show -s --date=short --pretty='format:%h ("%s", %ad)' <commit>
```

If a commit references an issue please add a reference to the body of your
commit message, e.g. `issue #1234` or `fixes #456`. Using keywords like
`fixes`, `resolves`, or `closes` will cause the corresponding issue to be
closed when the pull request is merged.
如果提交引用了问题，请添加对您的正文的引用提交消息，例如 `问题＃1234`或`修复＃456`。 
使用关键字`fixes`，`resolves`或`closing`将导致相应的问题合并拉取请求时关闭。

Avoid adding any `@` mentions to commit messages, instead add them to the PR
cover letter.
避免添加任何`@`提及来提交消息，而是将它们添加到PR封面信中。

## Responding to Reviewer Feedback 回应审稿人反馈

During the review process a reviewer may ask you to make changes to your pull
request. If a particular commit needs to be changed, that commit should be
amended directly. Changes in response to a review *should not* be made in
separate commits on top of your PR unless it logically makes sense to have
separate, distinct commits for those changes. This helps keep the commit
history clean.
在审核过程中，审核人可能会要求您更改您的提取请求。 如果需要更改特定提交，则应直接修改该提交。 
对审核*的响应变化不应该在PR的单独提交中进行，除非在逻辑上有意义的是对这些更改进行单独的，
不同的提交。 这有助于保持提交历史记录清洁。

If your pull request is out-of-date and needs to be updated because `master`
has advanced, you should rebase your branch on top of the latest master by
doing the following:
如果你的拉取请求已经过时并且需要更新，因为`master`已经提前，你应该通过执行以下操作
在最新的master之上重新分支你的分支：

```bash
git fetch upstream
git checkout topic
git rebase -i upstream/master
```

You *should not* update your branch by merging the latest master into your
branch. Merge commits included in PRs tend to make it more difficult for the
reviewer to understand the change being made, especially if the merge wasn't
clean and needed conflicts to be resolved. As such, PRs with merge commits will
be rejected.
您*不应*通过将最新的主服务器合并到您的分支中来更新您的分支。 PR中包含的合并提交往往会
使审阅者更难理解所做的更改，尤其是在合并不清晰且需要解决冲突的情况下。 因此，具有合并
提交的PR将被拒绝。

## Bisect-able History 可分辨历史

It is important that the project history is bisect-able so that when
regressions are identified we can easily use `git bisect` to be able to
pin-point the exact commit which introduced the regression. This requires that
every commit is able to be built and passes all lints and tests. So if your
pull request includes multiple commits be sure that each and every commit is
able to be built and passes all checks performed by CI.
重要的是，项目历史是可分割的，以便何时我们可以很容易地使用`git bisect`来确定回归
精确定位引入回归的确切提交。 这要求每个提交都能够构建并传递所有lints和测试。 因此，
如果您的pull请求包含多个提交，请确保能够构建每个提交并传递CI执行的所有检查。

## Contributor License Agreement  贡献者许可协议

For pull request to be accepted by any Libra projects, a CLA must be signed.
You will only need to do this once to work on any of Libra's open source
projects. Individuals contributing on their own behalf can sign the [Individual
CLA](https://github.com/libra/libra/blob/master/contributing/individual-cla.pdf).
If you are contributing on behalf of your employer, please ask them to sign the
[Corporate 
CLA](https://github.com/libra/libra/blob/master/contributing/corporate-cla.pdf).
对于任何Libra项目接受的pull请求，必须签署CLA。 您只需要这样做一次就可以在Libra的任何
开源项目上工作。 代表自己的个人可以签署
[个人CLA]（https://github.com/libra/libra/blob/master/contributing/individual-cla.pdf）。
如果您代表您的雇主做出贡献，请让他们签署
[Corporate CLA]（https://github.com/libra/libra/blob/master/contributing/corporate-cla.pdf）。

## Issues 问题

Libra uses [GitHub issues](https://github.com/libra/libra/issues) to track
bugs. Please include necessary information and instructions to reproduce your
issue.
Libra使用[GitHub问题]（https://github.com/libra/libra/issues）来跟踪
错误。 请包含重现您的必要信息和说明问题。
