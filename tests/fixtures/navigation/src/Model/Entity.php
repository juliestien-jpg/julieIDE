<?php
namespace App\Model;

abstract class Entity
{
    protected int $id = 0;

    public function getId(): int
    {
        return $this->id;
    }

    public function save(): void
    {
    }
}
