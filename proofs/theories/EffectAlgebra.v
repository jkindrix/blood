(** * Blood Core Calculus — Effect Row Algebra

    Algebraic properties of effect rows: subset, union, reflexivity,
    transitivity. Also includes lin_split_nil_inv for empty context
    splitting.

    Extracted from Preservation.v during modularization.
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Bool.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.

(** ** Helper: right inclusion in effect_entries_union *)

Lemma effect_entries_union_r :
  forall es1 es2 e,
    In e es2 -> In e (effect_entries_union es1 es2).
Proof.
  induction es1 as [| [n] rest IH]; intros es2 e Hin; simpl.
  - exact Hin.
  - destruct (existsb _ es2).
    + apply IH. exact Hin.
    + right. apply IH. exact Hin.
Qed.

(** ** Helper: left inclusion in effect_entries_union *)

Lemma effect_entries_union_l :
  forall es1 es2 e,
    In e es1 -> In e (effect_entries_union es1 es2).
Proof.
  induction es1 as [| [n] rest IH]; intros es2 e Hin; simpl.
  - destruct Hin.
  - destruct Hin as [Heq | Hin'].
    + subst. destruct (existsb _ es2) eqn:Hexists.
      * (* n already in es2, so in union via right inclusion *)
        apply existsb_exists in Hexists.
        destruct Hexists as [[n'] [Hin2 Heqb]].
        simpl in Heqb. apply String.eqb_eq in Heqb. subst.
        apply effect_entries_union_r. exact Hin2.
      * (* n not in es2, prepended *)
        left. reflexivity.
    + destruct (existsb _ es2).
      * apply IH. exact Hin'.
      * right. apply IH. exact Hin'.
Qed.

(** ** Effect row subset of union right component *)

Lemma effect_row_subset_union_r :
  forall e1 e2,
    effect_row_subset e2 (effect_row_union e1 e2).
Proof.
  intros [| es1 | es1 rv1] [| es2 | es2 rv2]; simpl.
  - (* Pure, Pure *) trivial.
  - (* Pure, Closed *) intros e Hin. exact Hin.
  - (* Pure, Open *) intros e Hin. exact Hin.
  - (* Closed, Pure *) trivial.
  - (* Closed, Closed *) intros e Hin. apply effect_entries_union_r. exact Hin.
  - (* Closed, Open *) intros e Hin. apply effect_entries_union_r. exact Hin.
  - (* Open, Pure *) trivial.
  - (* Open, Closed *) intros e Hin. apply effect_entries_union_r. exact Hin.
  - (* Open, Open *) intros e Hin. apply effect_entries_union_r. exact Hin.
Qed.

(** ** Effect row subset reflexivity *)

Lemma effect_row_subset_refl :
  forall eff, effect_row_subset eff eff.
Proof.
  destruct eff; simpl; auto.
Qed.

(** ** Effect row subset transitivity *)

Lemma effect_row_subset_trans :
  forall e1 e2 e3,
    effect_row_subset e1 e2 ->
    effect_row_subset e2 e3 ->
    effect_row_subset e1 e3.
Proof.
  intros e1 e2 e3 H12 H23.
  destruct e1; simpl in *.
  - (* e1 = Eff_Pure: always subset *)
    trivial.
  - (* e1 = Eff_Closed l *)
    destruct e2; simpl in *.
    + (* e2 = Eff_Pure: H12 says l = [] *)
      destruct e3; simpl in *.
      * (* e3 = Eff_Pure *) exact H12.
      * (* e3 = Eff_Closed *) rewrite H12. intros e Hin. inversion Hin.
      * (* e3 = Eff_Open *) rewrite H12. intros e Hin. inversion Hin.
    + (* e2 = Eff_Closed l0 *)
      destruct e3; simpl in *.
      * (* e3 = Eff_Pure: H23 says l0 = [] *)
        destruct l as [|hd tl].
        { reflexivity. }
        { exfalso. specialize (H12 hd (or_introl eq_refl)). rewrite H23 in H12. exact H12. }
      * (* e3 = Eff_Closed l1 *)
        intros e Hin. apply H23. apply H12. assumption.
      * (* e3 = Eff_Open l1 n *)
        intros e Hin. apply H23. apply H12. assumption.
    + (* e2 = Eff_Open l0 n *)
      destruct e3; simpl in *.
      * contradiction.
      * contradiction.
      * intros e Hin. apply H23. apply H12. assumption.
  - (* e1 = Eff_Open l n: can only be subset of open rows *)
    destruct e2; simpl in *.
    + contradiction.
    + contradiction.
    + destruct e3; simpl in *.
      * contradiction.
      * contradiction.
      * intros e Hin. apply H23. apply H12. assumption.
Qed.

(** ** Helper: lin_split of empty context forces both sides empty *)

Lemma lin_split_nil_inv :
  forall Delta1 Delta2,
    lin_split [] Delta1 Delta2 -> Delta1 = [] /\ Delta2 = [].
Proof.
  intros Delta1 Delta2 H. inversion H. auto.
Qed.

(** ** Effect subset of union left component *)

Lemma effect_row_subset_union_l :
  forall e1 e2,
    effect_row_subset e1 (effect_row_union e1 e2).
Proof.
  intros [| es1 | es1 rv1] [| es2 | es2 rv2]; simpl.
  - (* Pure, Pure *) trivial.
  - (* Pure, Closed *) trivial.
  - (* Pure, Open *) trivial.
  - (* Closed, Pure *) intros e Hin. exact Hin.
  - (* Closed, Closed *) intros e Hin. apply effect_entries_union_l. exact Hin.
  - (* Closed, Open *) intros e Hin. apply effect_entries_union_l. exact Hin.
  - (* Open, Pure *) intros e Hin. exact Hin.
  - (* Open, Closed *) intros e Hin. apply effect_entries_union_l. exact Hin.
  - (* Open, Open *) intros e Hin. apply effect_entries_union_l. exact Hin.
Qed.
