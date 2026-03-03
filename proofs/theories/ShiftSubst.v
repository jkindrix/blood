(** * Blood Core Calculus — Shift/Subst Commutation Lemmas

    Algebraic properties of shift and substitution:
    - shift_shift_commute: shifts at disjoint cutoffs commute
    - shift_subst_commute: shift and substitution commute
    - shift_then_subst_general: shift then subst cancels
    - subst_shift_cancel: corollary at position 0

    Note: shift_compose stays in Substitution.v because
    subst_preserves_typing depends on it (avoids circular import).

    Extracted from Substitution.v during modularization.
    Phase: M1 — Core Type System
*)

From Stdlib Require Import String.
From Stdlib Require Import List.
From Stdlib Require Import Arith.
From Stdlib Require Import Bool.
From Stdlib Require Import PeanoNat.
From Stdlib Require Import Lia.
Import ListNotations.
From Blood Require Import Syntax.
From Blood Require Import Typing.
From Blood Require Import Substitution.

(** ** Shifting commutes with shifting (disjoint cutoffs)

    When c1 <= c2, shifting by d1 at c1 then by d2 at c2+d1
    equals shifting by d2 at c2 then by d1 at c1. *)

Lemma shift_shift_commute :
  forall e d1 d2 c1 c2,
    c1 <= c2 ->
    shift_expr d2 (c2 + d1) (shift_expr d1 c1 e) =
    shift_expr d1 c1 (shift_expr d2 c2 e).
Proof.
  intro e.
  apply (expr_nested_ind
    (fun e => forall d1 d2 c1 c2, c1 <= c2 ->
       shift_expr d2 (c2 + d1) (shift_expr d1 c1 e) =
       shift_expr d1 c1 (shift_expr d2 c2 e))
    (fun h => forall d1 d2 c1 c2, c1 <= c2 ->
       shift_handler d2 (c2 + d1) (shift_handler d1 c1 h) =
       shift_handler d1 c1 (shift_handler d2 c2 h))
    (fun cl => forall d1 d2 c1 c2, c1 <= c2 ->
       shift_op_clause d2 (c2 + d1) (shift_op_clause d1 c1 cl) =
       shift_op_clause d1 c1 (shift_op_clause d2 c2 cl))).

  - (* E_Var *)
    intros v d1 d2 c1 c2 Hle. simpl.
    destruct (c1 <=? v) eqn:Hc1v; destruct (c2 <=? v) eqn:Hc2v; simpl.
    + (* c1 <= v, c2 <= v *)
      destruct (c2 + d1 <=? v + d1) eqn:H1; destruct (c1 <=? v + d2) eqn:H2.
      * f_equal. apply Nat.leb_le in Hc1v. lia.
      * exfalso. apply Nat.leb_le in Hc1v. apply Nat.leb_gt in H2. lia.
      * exfalso. apply Nat.leb_le in Hc2v. apply Nat.leb_gt in H1. lia.
      * exfalso. apply Nat.leb_le in Hc2v. apply Nat.leb_gt in H1. lia.
    + (* c1 <= v, c2 > v *)
      rewrite Hc1v. simpl.
      destruct (c2 + d1 <=? v + d1) eqn:H1.
      * exfalso. apply Nat.leb_le in H1. apply Nat.leb_gt in Hc2v. lia.
      * reflexivity.
    + (* c1 > v, c2 <= v: impossible *)
      exfalso. apply Nat.leb_gt in Hc1v. apply Nat.leb_le in Hc2v. lia.
    + (* c1 > v, c2 > v *)
      rewrite Hc1v. simpl.
      destruct (c2 + d1 <=? v) eqn:H1.
      * exfalso. apply Nat.leb_le in H1. apply Nat.leb_gt in Hc2v. lia.
      * reflexivity.

  - (* E_Const *) intros c d1 d2 c1 c2 Hle. reflexivity.

  - (* E_Lam *) intros T body IH d1 d2 c1 c2 Hle. simpl. f_equal.
    apply (IH d1 d2 (S c1) (S c2)). lia.

  - (* E_App *) intros e1 e2 IH1 IH2 d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Let *) intros e1 e2 IH1 IH2 d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IH1; assumption | apply (IH2 d1 d2 (S c1) (S c2)); lia].

  - (* E_Annot *) intros e0 T IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* E_Record *)
    intros fields HFA d1 d2 c1 c2 Hle. simpl. f_equal.
    induction HFA as [| [l' e'] rest He' _ IH].
    + reflexivity.
    + simpl in He'. simpl. rewrite (He' d1 d2 c1 c2 Hle).
      f_equal. exact IH.

  - (* E_Select *) intros e0 l IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* E_Extend *) intros l e1 e2 IH1 IH2 d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Perform *) intros eff op e0 IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* E_Handle *) intros h e0 IHh IHe d1 d2 c1 c2 Hle. simpl.
    f_equal; [apply IHh | apply IHe]; assumption.

  - (* E_Resume *) intros e0 IH d1 d2 c1 c2 Hle. simpl.
    f_equal. apply IH. assumption.

  - (* Handler *)
    intros hk e_ret clauses IHret IHclauses d1 d2 c1 c2 Hle. simpl.
    f_equal.
    + apply (IHret d1 d2 (S c1) (S c2)). lia.
    + induction IHclauses as [| cl rest Hcl _ IH].
      * reflexivity.
      * simpl. rewrite (Hcl d1 d2 c1 c2 Hle). f_equal. exact IH.

  - (* OpClause *)
    intros eff op body IH d1 d2 c1 c2 Hle. simpl. f_equal.
    apply (IH d1 d2 (S (S c1)) (S (S c2))). lia.
Qed.

(** ** Substitution commutes with shifting *)

Lemma shift_subst_commute :
  forall e d c j s,
    c <= j ->
    shift_expr d c (subst j s e) =
    subst (j + d) (shift_expr d c s) (shift_expr d c e).
Proof.
  intro e.
  apply (expr_nested_ind
    (fun e => forall d c j s, c <= j ->
       shift_expr d c (subst j s e) =
       subst (j + d) (shift_expr d c s) (shift_expr d c e))
    (fun h => forall d c j s, c <= j ->
       shift_handler d c (subst_handler j s h) =
       subst_handler (j + d) (shift_expr d c s) (shift_handler d c h))
    (fun cl => forall d c j s, c <= j ->
       shift_op_clause d c (subst_op_clause j s cl) =
       subst_op_clause (j + d) (shift_expr d c s) (shift_op_clause d c cl))).

  - (* E_Var *)
    intros v dd cc jj ss Hcj. simpl.
    destruct (jj =? v) eqn:Hjv.
    + (* jj = v *)
      apply Nat.eqb_eq in Hjv. subst v.
      simpl.
      destruct (cc <=? jj) eqn:Hccj.
      * simpl.
        destruct (jj + dd =? jj + dd) eqn:Heq.
        { reflexivity. }
        { apply Nat.eqb_neq in Heq. lia. }
      * apply Nat.leb_gt in Hccj. lia.
    + (* jj <> v *)
      destruct (jj <? v) eqn:Hjv'.
      * (* jj < v *)
        apply Nat.ltb_lt in Hjv'.
        simpl.
        destruct (cc <=? v - 1) eqn:Hcv1.
        { apply Nat.leb_le in Hcv1. simpl.
          destruct (cc <=? v) eqn:Hcv.
          { simpl.
            destruct (jj + dd =? v + dd) eqn:Hjdvd.
            { apply Nat.eqb_eq in Hjdvd. apply Nat.eqb_neq in Hjv. lia. }
            destruct (jj + dd <? v + dd) eqn:Hltjv.
            { f_equal. lia. }
            { apply Nat.ltb_ge in Hltjv. lia. }
          }
          { apply Nat.leb_gt in Hcv. lia. }
        }
        { apply Nat.leb_gt in Hcv1. simpl.
          destruct (cc <=? v) eqn:Hcv.
          { apply Nat.leb_le in Hcv. assert (cc = v) by lia. subst cc. simpl.
            destruct (jj + dd =? v + dd) eqn:Hjdvd.
            { apply Nat.eqb_eq in Hjdvd. apply Nat.eqb_neq in Hjv. lia. }
            destruct (jj + dd <? v + dd) eqn:Hltjv.
            { f_equal. lia. }
            { apply Nat.ltb_ge in Hltjv. lia. }
          }
          { apply Nat.leb_gt in Hcv. lia. }
        }
      * (* jj >= v *)
        apply Nat.ltb_ge in Hjv'. apply Nat.eqb_neq in Hjv.
        assert (jj > v) by lia. simpl.
        destruct (cc <=? v) eqn:Hcv.
        { simpl.
          destruct (jj + dd =? v + dd) eqn:Hjdvd.
          { apply Nat.eqb_eq in Hjdvd. lia. }
          destruct (jj + dd <? v + dd) eqn:Hltjv.
          { apply Nat.ltb_lt in Hltjv. lia. }
          { reflexivity. }
        }
        { simpl.
          destruct (jj + dd =? v) eqn:Hjdv.
          { apply Nat.eqb_eq in Hjdv. lia. }
          destruct (jj + dd <? v) eqn:Hltjdv.
          { apply Nat.ltb_lt in Hltjdv. lia. }
          { reflexivity. }
        }

  - (* E_Const *) intros c dd cc jj ss Hcj. reflexivity.

  - (* E_Lam *)
    intros T body IH dd cc jj ss Hcj. simpl. f_equal.
    rewrite (IH dd (S cc) (S jj) (shift_expr 1 0 ss) ltac:(lia)).
    f_equal. replace (S cc) with (cc + 1) by lia.
    apply shift_shift_commute. lia.

  - (* E_App *)
    intros e1 e2 IH1 IH2 dd cc jj ss Hcj. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Let *)
    intros e1 e2 IH1 IH2 dd cc jj ss Hcj. simpl. f_equal.
    + apply IH1. assumption.
    + rewrite (IH2 dd (S cc) (S jj) (shift_expr 1 0 ss) ltac:(lia)).
      f_equal. replace (S cc) with (cc + 1) by lia.
      apply shift_shift_commute. lia.

  - (* E_Annot *)
    intros e0 T IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* E_Record *)
    intros fields HFA dd cc jj ss Hcj. simpl. f_equal.
    induction HFA as [| [l' e'] rest He' _ IH].
    + reflexivity.
    + simpl in He'. simpl. rewrite (He' dd cc jj ss Hcj).
      f_equal. exact IH.

  - (* E_Select *)
    intros e0 l IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* E_Extend *)
    intros l e1 e2 IH1 IH2 dd cc jj ss Hcj. simpl.
    f_equal; [apply IH1 | apply IH2]; assumption.

  - (* E_Perform *)
    intros eff op e0 IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* E_Handle *)
    intros h e0 IHh IHe dd cc jj ss Hcj. simpl.
    f_equal; [apply IHh | apply IHe]; assumption.

  - (* E_Resume *)
    intros e0 IH dd cc jj ss Hcj. simpl. f_equal. apply IH. assumption.

  - (* Handler *)
    intros hk e_ret clauses IHret IHclauses dd cc jj ss Hcj. simpl.
    f_equal.
    + rewrite (IHret dd (S cc) (S jj) (shift_expr 1 0 ss) ltac:(lia)).
      f_equal. replace (S cc) with (cc + 1) by lia.
      apply shift_shift_commute. lia.
    + induction IHclauses as [| cl rest Hcl _ IH].
      * reflexivity.
      * simpl. rewrite (Hcl dd cc jj ss Hcj). f_equal. exact IH.

  - (* OpClause *)
    intros eff op body IH dd cc jj ss Hcj. simpl. f_equal.
    rewrite (IH dd (S (S cc)) (S (S jj)) (shift_expr 2 0 ss) ltac:(lia)).
    f_equal. replace (S (S cc)) with (cc + 2) by lia.
    apply shift_shift_commute. lia.
Qed.

(** ** Generalized lemma: shifting then substituting cancels *)

Lemma shift_then_subst_general :
  forall e s cutoff,
    subst cutoff s (shift_expr 1 cutoff e) = e.
Proof.
  intro e.
  apply (expr_nested_ind
    (fun e => forall s cutoff, subst cutoff s (shift_expr 1 cutoff e) = e)
    (fun h => forall s cutoff,
       subst_handler cutoff s (shift_handler 1 cutoff h) = h)
    (fun cl => forall s cutoff,
       subst_op_clause cutoff s (shift_op_clause 1 cutoff cl) = cl)).

  - (* E_Var *)
    intros v s cutoff. simpl.
    destruct (cutoff <=? v) eqn:Hle.
    + simpl.
      destruct (cutoff =? v + 1) eqn:Heq.
      * apply Nat.eqb_eq in Heq. apply Nat.leb_le in Hle. lia.
      * destruct (cutoff <? v + 1) eqn:Hlt.
        { f_equal. apply Nat.leb_le in Hle. lia. }
        { apply Nat.ltb_ge in Hlt. apply Nat.leb_le in Hle. lia. }
    + simpl.
      destruct (cutoff =? v) eqn:Heq.
      * apply Nat.eqb_eq in Heq. apply Nat.leb_gt in Hle. lia.
      * destruct (cutoff <? v) eqn:Hlt.
        { apply Nat.ltb_lt in Hlt. apply Nat.leb_gt in Hle. lia. }
        { reflexivity. }

  - (* E_Const *) intros c s cutoff. reflexivity.
  - (* E_Lam *) intros T body IH s cutoff. simpl. f_equal. apply IH.
  - (* E_App *) intros e1 e2 IH1 IH2 s cutoff. simpl.
    f_equal; [apply IH1 | apply IH2].
  - (* E_Let *) intros e1 e2 IH1 IH2 s cutoff. simpl.
    f_equal; [apply IH1 | apply IH2].
  - (* E_Annot *) intros e0 T IH s cutoff. simpl. f_equal. apply IH.

  - (* E_Record *)
    intros fields HFA s cutoff. simpl. f_equal.
    induction HFA as [| [l' e'] rest He' _ IH].
    + reflexivity.
    + simpl in He'. simpl. rewrite (He' s cutoff). f_equal. exact IH.

  - (* E_Select *) intros e0 l IH s cutoff. simpl. f_equal. apply IH.
  - (* E_Extend *) intros l e1 e2 IH1 IH2 s cutoff. simpl.
    f_equal; [apply IH1 | apply IH2].
  - (* E_Perform *) intros eff op e0 IH s cutoff. simpl.
    f_equal. apply IH.
  - (* E_Handle *) intros h e0 IHh IHe s cutoff. simpl.
    f_equal; [apply IHh | apply IHe].
  - (* E_Resume *) intros e0 IH s cutoff. simpl. f_equal. apply IH.

  - (* Handler *)
    intros hk e_ret clauses IHret IHclauses s cutoff. simpl. f_equal.
    + apply IHret.
    + induction IHclauses as [| cl rest Hcl _ IH].
      * reflexivity.
      * simpl. rewrite (Hcl s cutoff). f_equal. exact IH.

  - (* OpClause *)
    intros eff op body IH s cutoff. simpl. f_equal. apply IH.
Qed.

(** ** Identity substitution: shift then subst at 0 *)

Lemma subst_shift_cancel :
  forall e v,
    subst 0 v (shift_expr 1 0 e) = e.
Proof.
  intros e v.
  apply shift_then_subst_general.
Qed.
